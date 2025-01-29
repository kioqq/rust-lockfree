use std::sync::atomic::{AtomicBool, AtomicPtr, AtomicUsize, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

/// Демонстрация использования `AtomicBool`
/// Используется для передачи флага между потоками без блокировок
pub fn atomic_bool_example() {
    // Создаём атомарный флаг с начальным значением `false`
    let ready = Arc::new(AtomicBool::new(false));

    let r = Arc::clone(&ready);
    let handle = thread::spawn(move || {
        // Ожидаем, пока значение `ready` не станет `true`
        while !r.load(Ordering::Acquire) {
            println!("Ожидание...");
            thread::sleep(Duration::from_millis(500)); // Имитируем работу
        }
        println!("Готово!");
    });

    // Через 1 секунду меняем флаг на `true`, разрешая второму потоку продолжить
    thread::sleep(Duration::from_secs(1));
    ready.store(true, Ordering::Release);

    // Дожидаемся завершения потока
    handle.join().unwrap();
}

/// Демонстрация `AtomicUsize`
/// Используется как потокобезопасный счётчик без мьютексов
pub fn atomic_usize_example() {
    let counter = Arc::new(AtomicUsize::new(0));
    let mut handles = vec![];

    for _ in 0..4 {
        let c = Arc::clone(&counter);
        let handle = thread::spawn(move || {
            for _ in 0..10 {
                // Атомарное увеличение счётчика на 1
                c.fetch_add(1, Ordering::Relaxed);
            }
        });
        handles.push(handle);
    }

    for handle in handles {
        handle.join().unwrap();
    }

    // Ожидаемое финальное значение: 4 * 10 = 40
    println!("Финальное значение: {}", counter.load(Ordering::Relaxed));
}

/// Демонстрация `AtomicPtr`
/// Используется для работы с указателями в lock-free структурах данных
pub fn atomic_ptr_example() {
    let mut data = 42; // Исходное значение
    let atomic_ptr = AtomicPtr::new(&mut data);

    // Получаем значение по указателю
    let loaded = atomic_ptr.load(Ordering::SeqCst);
    unsafe {
        println!("Значение: {}", *loaded);
    }

    // Меняем указатель на новый объект
    let mut new_data = 100;
    atomic_ptr.store(&mut new_data, Ordering::SeqCst);

    let new_loaded = atomic_ptr.load(Ordering::SeqCst);
    unsafe {
        println!("Новое значение: {}", *new_loaded);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_atomic_bool_example() {
        atomic_bool_example();
    }

    #[test]
    fn test_atomic_usize_example() {
        atomic_usize_example();
    }

    #[test]
    fn test_atomic_ptr_example() {
        atomic_ptr_example();
    }
}
