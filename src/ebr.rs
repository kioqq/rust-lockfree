use std::{
    cell::RefCell,
    collections::VecDeque,
    sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        Mutex,
    },
};

/// Максимальное число потоков.
/// В реальном (промышленном) коде обычно динамически расширяемо
/// (например, хранится в Vec или hashmap).
const MAX_THREADS: usize = 32;

/// Значение, указывающее, что поток "не прикреплён" (unpinned).
/// Если `local_epoch == UNPINNED_EPOCH`, значит поток не находится
/// в активном чтении (не держит объекты).
const UNPINNED_EPOCH: usize = usize::MAX;

/// Глобальная структура EBR, хранимая в статике.
/// Состоит из:
/// 1) global_epoch — текущее значение "эпохи".
/// 2) threads — массив (до 32 слотов) для регистрации потоков.
/// 3) epoch_lock — мьютекс для управления продвижением эпохи.
struct GlobalEBR {
    global_epoch: AtomicUsize,
    threads: [EBRThread; MAX_THREADS],
    epoch_lock: Mutex<()>,
}

/// Структура, описывающая один слот потока (EBRThread):
/// - active — флаг, зарегистрирован ли поток.
/// - local_epoch — актуальная эпоха, на которую поток "закрепился" при pin().
struct EBRThread {
    active: AtomicBool,
    local_epoch: AtomicUsize,
}

/// Глобальная переменная с ленивой (Lazy) инициализацией.
/// При первом обращении инициализируется массивом 32 слотов.
static GLOBAL_EBR: once_cell::sync::Lazy<GlobalEBR> = once_cell::sync::Lazy::new(|| GlobalEBR {
    global_epoch: AtomicUsize::new(0),
    threads: array_init::array_init(|_| EBRThread {
        active: AtomicBool::new(false),
        local_epoch: AtomicUsize::new(UNPINNED_EPOCH),
    }),
    epoch_lock: Mutex::new(()),
});

/// Данные, которые мы "откладываем" (retire) для отложенного освобождения:
/// - ptr: сырая ссылка (*mut ()) на объект,
/// - deleter: функция, которая умеет освободить ptr,
/// - epoch: эпоха, когда объект был помечен на retire.
struct Retired {
    ptr: *mut (),
    deleter: fn(*mut ()),
    epoch: usize,
}

/// Локальные данные, хранимые в thread_local:
/// - index: номер слота в GLOBAL_EBR.threads,
/// - local_garbage: очередь объектов, отложенных к освобождению.
#[derive(Default)]
struct LocalData {
    index: usize,
    local_garbage: VecDeque<Retired>,
}

// thread_local! хранит Option<LocalData> для каждого потока.
// Если None, значит поток ещё не зарегистрирован.
thread_local! {
    static LOCAL_DATA: RefCell<Option<LocalData>> = RefCell::new(None);
}

/// Guard, возвращаемый из `pin()`.
/// Пока существует Guard, поток считается "pinned":
/// - local_epoch != UNPINNED_EPOCH.
/// При дропе Guard делаем `unpin()` (local_epoch = UNPINNED_EPOCH).
pub struct Guard {
    index: usize,
    epoch: usize,
}

impl Drop for Guard {
    fn drop(&mut self) {
        // При уничтожении Guard поток "отпинывается"
        let thr = &GLOBAL_EBR.threads[self.index];
        thr.local_epoch.store(UNPINNED_EPOCH, Ordering::Release);
    }
}

impl Guard {
    /// Возвращает локальную эпоху, на которую "закрепился" поток.
    pub fn epoch(&self) -> usize {
        self.epoch
    }
}

/// Функция auto_register_thread:
/// Автоматически ищет свободный слот в GLOBAL_EBR.threads, активирует его
/// и инициализирует локальную структуру (LocalData) в thread_local.
fn auto_register_thread() -> usize {
    for i in 0..MAX_THREADS {
        let thr = &GLOBAL_EBR.threads[i];
        // Сравнение: active == false => true
        // Если удалось, значит этот слот "наше".
        if thr
            .active
            .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .is_ok()
        {
            thr.local_epoch.store(UNPINNED_EPOCH, Ordering::Relaxed);

            // Создаём LocalData и кладём в thread_local
            let ld = LocalData {
                index: i,
                local_garbage: VecDeque::new(),
            };
            LOCAL_DATA.with(|l| {
                *l.borrow_mut() = Some(ld);
            });

            return i;
        }
    }
    panic!("No free slot in EBR threads (increase MAX_THREADS)");
}

/// pin():
/// 1) Если поток ещё не зарегистрирован, вызываем auto_register_thread().
/// 2) Считываем global_epoch.
/// 3) Записываем local_epoch = global_epoch (тем самым "закрепляемся").
/// Возвращаем Guard, который при дропе unpin'ит поток.
pub fn pin() -> Guard {
    let thr_id = LOCAL_DATA.with(|ld| {
        if ld.borrow().is_none() {
            // Автоматическая регистрация
            auto_register_thread();
        }
        let local_data = ld.borrow();
        local_data.as_ref().unwrap().index
    });

    let global_epoch = GLOBAL_EBR.global_epoch.load(Ordering::Acquire);
    let thr = &GLOBAL_EBR.threads[thr_id];
    thr.local_epoch.store(global_epoch, Ordering::Release);

    Guard {
        index: thr_id,
        epoch: global_epoch,
    }
}

/// retire():
/// Откладываем указатель ptr в локальный "мусор" (local_garbage).
/// Если там стало много (>=64), вызываем attempt_advance_epoch().
pub fn retire<T>(ptr: *mut T, deleter: fn(*mut T), guard: &Guard) {
    LOCAL_DATA.with(|ld| {
        let mut data = ld.borrow_mut();
        let ldref = data.as_mut().expect("auto_register_thread failed?");

        let r = Retired {
            ptr: ptr as *mut (),
            deleter: unsafe { std::mem::transmute::<fn(*mut T), fn(*mut ())>(deleter) },
            epoch: guard.epoch,
        };
        ldref.local_garbage.push_back(r);

        // Если накопилось много объектов — пробуем продвинуть эпоху
        if ldref.local_garbage.len() >= 64 {
            attempt_advance_epoch(ldref);
        }
    });
}

/// attempt_advance_epoch(ld):
/// 1) Берём epoch_lock.
/// 2) Проверяем, нет ли потока, который застрял на меньшей эпохе.
///    - Если есть, выходим.
/// 3) Иначе увеличиваем global_epoch.
/// 4) Освобождаем объекты в local_garbage, для которых r.epoch + 1 < new_epoch.
fn attempt_advance_epoch(ld: &mut LocalData) {
    let _lock = GLOBAL_EBR.epoch_lock.lock().unwrap();

    let cur_epoch = GLOBAL_EBR.global_epoch.load(Ordering::Acquire);

    // Проверяем все потоки, если кто-то pinned на старой эпохе (< cur_epoch), выходим
    for i in 0..MAX_THREADS {
        let thr = &GLOBAL_EBR.threads[i];
        if thr.active.load(Ordering::Relaxed) {
            let le = thr.local_epoch.load(Ordering::Acquire);
            if le != UNPINNED_EPOCH && le < cur_epoch {
                // Кто-то ещё держит старую эпоху => нельзя освобождать
                return;
            }
        }
    }

    // Если все >= cur_epoch => можно сдвинуть
    let new_epoch = cur_epoch.wrapping_add(1);
    GLOBAL_EBR.global_epoch.store(new_epoch, Ordering::Release);

    // Удаляем объекты из local_garbage, у которых epoch + 1 < new_epoch
    let mut i = 0;
    while i < ld.local_garbage.len() {
        if ld.local_garbage[i].epoch + 1 < new_epoch {
            let r = ld.local_garbage.remove(i).unwrap();
            (r.deleter)(r.ptr);
        } else {
            i += 1;
        }
    }
}

/// Опционально — unregister_thread():
/// 1) Ставим active = false,
/// 2) local_epoch = UNPINNED_EPOCH,
/// 3) Освобождаем все объекты, накопленные в local_garbage.
///
/// Это нужно, если поток заканчивает работу и мы хотим сразу очистить
/// локальный буфер, не дожидаясь продвижения эпохи.
pub fn unregister_thread() {
    LOCAL_DATA.with(|ld| {
        if let Some(mut data) = ld.borrow_mut().take() {
            // поток более не активен
            let thr = &GLOBAL_EBR.threads[data.index];
            thr.active.store(false, Ordering::Release);
            thr.local_epoch.store(UNPINNED_EPOCH, Ordering::Release);

            // Освобождаем все объекты
            while let Some(r) = data.local_garbage.pop_front() {
                (r.deleter)(r.ptr);
            }
        }
    });
}
