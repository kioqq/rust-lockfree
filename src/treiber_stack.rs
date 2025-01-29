use std::ptr;
use std::sync::atomic::{AtomicPtr, Ordering};
// use std::thread;
// use std::sync::Arc;

/// Узел стека (односвязный список)
struct Node<T> {
    value: T,
    next: *mut Node<T>, // Указатель на следующий узел
}

/// Lock-Free стек (Treiber Stack)
pub struct TreiberStack<T> {
    head: AtomicPtr<Node<T>>, // Атомарный указатель на верхний элемент стека
}

impl<T> TreiberStack<T> {
    /// Создаёт новый пустой стек
    pub fn new() -> Self {
        TreiberStack {
            head: AtomicPtr::new(ptr::null_mut()), // Начальный стек пуст
        }
    }

    /// Добавляет элемент в стек (lock-free push)
    pub fn push(&self, value: T) {
        let new_node = Box::into_raw(Box::new(Node {
            value,
            next: ptr::null_mut(),
        }));

        loop {
            let head = self.head.load(Ordering::Acquire); // Загружаем текущий верхний элемент
            unsafe { (*new_node).next = head }; // Новый узел указывает на старый head

            // Атомарно обновляем head, если никто другой не изменил стек
            if self
                .head
                .compare_exchange_weak(head, new_node, Ordering::Release, Ordering::Relaxed)
                .is_ok()
            {
                break; // Успешная вставка, выходим из цикла
            }
        }
    }

    /// Удаляет и возвращает верхний элемент из стека (lock-free pop)
    pub fn pop(&self) -> Option<T> {
        loop {
            let head = self.head.load(Ordering::Acquire); // Загружаем текущий head
            if head.is_null() {
                return None; // Стек пуст
            }

            let next = unsafe { (*head).next }; // Получаем следующий элемент

            // Атомарно обновляем head, если никто другой не изменил стек
            if self
                .head
                .compare_exchange_weak(head, next, Ordering::Release, Ordering::Relaxed)
                .is_ok()
            {
                let node = unsafe { Box::from_raw(head) }; // Безопасно забираем узел
                return Some(node.value); // Возвращаем значение узла
            }
        }
    }
}

impl<T> Drop for TreiberStack<T> {
    /// Освобождает всю память при уничтожении стека
    fn drop(&mut self) {
        while self.pop().is_some() {} // Очищаем стек
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::thread;

    #[test]
    fn test_single_thread() {
        let stack = TreiberStack::new();

        stack.push(1);
        stack.push(2);
        stack.push(3);

        assert_eq!(stack.pop(), Some(3));
        assert_eq!(stack.pop(), Some(2));
        assert_eq!(stack.pop(), Some(1));
        assert_eq!(stack.pop(), None); // Стек пуст
    }

    #[test]
    fn test_multi_threaded() {
        let stack = Arc::new(TreiberStack::new());
        let mut handles = vec![];
        let threads = 4;
        let iterations = 10;

        for i in 0..threads {
            let stack_clone = Arc::clone(&stack);
            let handle = thread::spawn(move || {
                for j in 0..iterations {
                    stack_clone.push(i * 10 + j);
                }
            });
            handles.push(handle);
        }

        for handle in handles {
            handle.join().unwrap();
        }

        let mut values = vec![];
        while let Some(value) = stack.pop() {
            values.push(value);
        }

        // Проверяем, что все 40 элементов добавились (4 потока * 10 элементов)
        assert_eq!(values.len(), threads * iterations);
    }
}
