use crossbeam_epoch::{self as epoch, Atomic, Owned, Shared};
use std::sync::atomic::Ordering;

/// Узел очереди (каждый узел хранит:
///  - data: Option<T> (None у фиктивного узла),
///  - next: атомарный указатель на следующий узел).
struct Node<T> {
    data: Option<T>,
    next: Atomic<Node<T>>,
}

impl<T> Node<T> {
    /// Конструктор узла с реальным значением
    fn new(data: T) -> Self {
        Self {
            data: Some(data),
            next: Atomic::null(),
        }
    }

    /// Создаём «фиктивный» (dummy) узел, у которого data == None.
    /// Он используется в начале очереди (head = tail = dummy).
    fn dummy() -> Self {
        Self {
            data: None,
            next: Atomic::null(),
        }
    }
}

/// Michael-Scott Queue
///
///  - `head`: указывает на первый узел (Atomic<Node<T>>)  
///  - `tail`: указывает на последний узел
///
/// Изначально head=tail указывают на dummy-узел.
pub struct MSQueue<T> {
    head: Atomic<Node<T>>,
    tail: Atomic<Node<T>>,
}

impl<T> MSQueue<T> {
    /// Создаём новую очередь: head и tail ссылаются на фиктивный (dummy) узел.
    pub fn new() -> Self {
        let guard = &epoch::pin();
        // "pin()" означает, что мы входим в критическую секцию Epoch.

        // Создаём dummy-узел в куче.
        let dummy = Owned::new(Node::dummy()).into_shared(guard); // Превращаем Owned в Shared под управлением Epoch

        MSQueue {
            // Инициализируем head и tail указателями на dummy-узел
            head: Atomic::from(dummy),
            tail: Atomic::from(dummy),
        }
    }

    /// Помещаем (enqueue) элемент в конец очереди.
    /// Реализуется классической MS-Queue логикой: пытаемся
    /// «приделать» новый узел к `tail.next`.
    pub fn push(&self, data: T) {
        let guard = &epoch::pin();
        // Каждый раз при работе с Queue мы входим в критическую секцию (pin).

        // Создаём новый узел в куче, сразу переводим Owned -> Shared.
        let new_node = Owned::new(Node::new(data)).into_shared(guard);

        loop {
            // Читаем текущий tail
            let tail = self.tail.load(Ordering::Acquire, guard);
            let tail_ref = unsafe { tail.deref() };
            // tail_ref — это «разыменованный» узел tail

            // Смотрим tail_ref.next (указатель на следующий узел).
            let next = tail_ref.next.load(Ordering::Acquire, guard);

            if !next.is_null() {
                // Если next не пуст, значит кто-то уже добавил новый узел,
                // но tail ещё не сдвинулся. Помогаем сдвинуть tail вперёд.
                let _ = self.tail.compare_exchange_weak(
                    tail,
                    next,
                    Ordering::Release,
                    Ordering::Relaxed,
                    guard,
                );
                // Идём заново в loop.
                continue;
            }

            // Если next == null, значит tail действительно указывает
            // на «последний» узел. Пытаемся прицепить new_node туда.
            if tail_ref
                .next
                .compare_exchange_weak(
                    Shared::null(), // Ожидаем, что там null
                    new_node,       // хотим поставить new_node
                    Ordering::Release,
                    Ordering::Relaxed,
                    guard,
                )
                .is_ok()
            {
                // Успешно прицепили new_node к tail.next.
                // Теперь пытаемся сдвинуть tail на new_node (не критично, если не выйдет).
                let _ = self.tail.compare_exchange_weak(
                    tail,
                    new_node,
                    Ordering::Release,
                    Ordering::Relaxed,
                    guard,
                );
                return; // Завершаем push.
            }
            // Если compare_exchange не сработал, значит кто-то нас опередил, повторяем loop.
        }
    }

    /// Извлекаем (dequeue) элемент из головы.
    /// Возвращаем Some(T), если очередь не пуста, или None, если пуста.
    pub fn pop(&self) -> Option<T> {
        let guard = &epoch::pin();

        loop {
            // Загружаем head
            let head = self.head.load(Ordering::Acquire, guard);
            let head_ref = unsafe { head.deref() };

            // Смотрим head_ref.next — если пуст, значит очередь пуста.
            let next = head_ref.next.load(Ordering::Acquire, guard);

            if next.is_null() {
                return None;
            }

            // Если next не пуст, пробуем сдвинуть head → next
            if self
                .head
                .compare_exchange_weak(head, next, Ordering::Release, Ordering::Relaxed, guard)
                .is_ok()
            {
                // Успешно поменяли head.
                // Узел head больше не нужен, откладываем его удаление.
                let next_ptr = next.as_raw() as *mut Node<T>;
                // Получаем мутабельную ссылку
                let next_ref = unsafe { &mut *next_ptr };
                // Забираем данные (Some(T)) из нового head
                let data = next_ref.data.take();

                // Откладываем освобождение старого head:
                unsafe {
                    guard.defer_destroy(head);
                }

                return data;
            }
            // Если compare_exchange не сработал, значит head поменялся, повторяем loop.
        }
    }
}

/// Drop-логика: очищаем все элементы из очереди, пока есть.
/// Затем удаляем dummy-узел через `defer_destroy`.
impl<T> Drop for MSQueue<T> {
    fn drop(&mut self) {
        // Пока есть элементы, достаём их
        while self.pop().is_some() {}

        let guard = &epoch::pin();
        // Освобождаем dummy-узел
        let head = self.head.load(Ordering::Relaxed, guard);
        unsafe {
            guard.defer_destroy(head);
        }
    }
}

// --- Тесты ---
#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::thread;
    use std::time::Duration;

    /// 1. Проверка, что очередь пуста по умолчанию
    #[test]
    fn test_empty_queue() {
        let q: MSQueue<i32> = MSQueue::new();
        assert_eq!(q.pop(), None, "Новая очередь должна быть пустой");
    }

    /// 2. Тест, что мы можем добавить и убрать один элемент
    #[test]
    fn test_single_element() {
        let q: MSQueue<i32> = MSQueue::new();
        q.push(42);
        assert_eq!(q.pop(), Some(42));
        assert_eq!(q.pop(), None);
    }

    /// 3. Последовательные операции в одном потоке
    #[test]
    fn test_sequential_operations_single_thread() {
        let q: MSQueue<i32> = MSQueue::new();

        // Добавляем три элемента
        q.push(10);
        q.push(20);
        q.push(30);

        // Извлекаем два
        assert_eq!(q.pop(), Some(10));
        assert_eq!(q.pop(), Some(20));

        // Добавляем ещё
        q.push(40);

        // Проверяем остатки
        assert_eq!(q.pop(), Some(30));
        assert_eq!(q.pop(), Some(40));
        assert_eq!(q.pop(), None); // Пусто
    }

    /// 4. Многопоточность: 2 производителя, 2 потребителя
    #[test]
    fn test_2_producers_2_consumers() {
        let q = Arc::new(MSQueue::new());
        let mut handles = vec![];

        // Два производителя
        for i in 0..2 {
            let qc = Arc::clone(&q);
            handles.push(thread::spawn(move || {
                for j in 0..100 {
                    qc.push(i * 100 + j);
                }
            }));
        }

        // Два потребителя
        let results = Arc::new(std::sync::Mutex::new(Vec::new()));
        for _ in 0..2 {
            let qc = Arc::clone(&q);
            let res = Arc::clone(&results);
            handles.push(thread::spawn(move || {
                loop {
                    if let Some(val) = qc.pop() {
                        res.lock().unwrap().push(val);
                    } else {
                        // Если очередь пуста, подождём
                        thread::sleep(Duration::from_micros(50));
                        // Проверяем, достигли ли мы 200
                        if res.lock().unwrap().len() >= 200 {
                            break;
                        }
                    }
                }
            }));
        }

        // Ждём все потоки
        for h in handles {
            h.join().unwrap();
        }

        let mut final_vec = results.lock().unwrap().clone();
        final_vec.sort();
        assert_eq!(final_vec.len(), 200);
        for i in 0..200 {
            assert_eq!(final_vec[i], i as i32);
        }
    }

    /// 5. Проверяем mix: 4 потока, в каждом чередуется push/pop
    #[test]
    fn test_mix_push_pop() {
        let q = Arc::new(MSQueue::new());
        let threads = 4;
        let iters = 500;
        let mut handles = vec![];

        // Каждый поток: в цикле (push + pop), многократно
        for t in 0..threads {
            let qc = Arc::clone(&q);
            handles.push(thread::spawn(move || {
                let start = t * 1000;
                for i in 0..iters {
                    qc.push(start + i);
                    let _ = qc.pop();
                }
            }));
        }

        for h in handles {
            h.join().unwrap();
        }

        // Сколько осталось — не знаем, но проверяем, что программа не падает
        let mut values = Vec::new();
        while let Some(x) = q.pop() {
            values.push(x);
        }
        // Можно сделать какие-то asserts, но результат не гарантирован
        // (так как concurrent).
    }

    /// 6. Stress-тест: много push/pop подряд разными потоками
    #[test]
    fn test_stress() {
        let q = Arc::new(MSQueue::new());

        let thread_count = 8;
        let per_thread = 2000;
        let mut handles = vec![];

        for _ in 0..thread_count {
            let qc = Arc::clone(&q);
            handles.push(thread::spawn(move || {
                for i in 0..per_thread {
                    qc.push(i);
                    // Сразу пытаемся pop()
                    let _ = qc.pop();
                }
            }));
        }

        for h in handles {
            h.join().unwrap();
        }

        let mut leftover = 0;
        while let Some(_val) = q.pop() {
            leftover += 1;
        }
        println!("Осталось {} элементов после stress-теста.", leftover);
    }
}
