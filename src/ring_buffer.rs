use std::cell::UnsafeCell;
use std::sync::atomic::{AtomicUsize, Ordering};

/// A lock-free ring buffer implementation with fixed capacity.
/// This structure is thread-safe and can be used in concurrent environments.
pub struct RingBuffer<T> {
    buffer: Vec<UnsafeCell<Option<T>>>, // Internal storage for data
    size: usize,                        // Fixed capacity of the buffer
    write_index: AtomicUsize,           // Write pointer
    read_index: AtomicUsize,            // Read pointer
}

// Ensure the buffer is thread-safe by implementing Sync
unsafe impl<T> Sync for RingBuffer<T> {}

impl<T> RingBuffer<T> {
    /// Creates a new `RingBuffer` with the given fixed size.
    ///
    /// # Arguments
    ///
    /// * `size` - The maximum number of elements the buffer can hold.
    ///
    /// # Returns
    ///
    /// A new instance of `RingBuffer`.
    pub fn new(size: usize) -> Self {
        let mut buffer = Vec::with_capacity(size);
        for _ in 0..size {
            buffer.push(UnsafeCell::new(None)); // Initialize the buffer with empty cells
        }

        RingBuffer {
            buffer,
            size,
            write_index: AtomicUsize::new(0), // Start with write index at 0
            read_index: AtomicUsize::new(0),  // Start with read index at 0
        }
    }

    /// Pushes an element into the buffer.
    ///
    /// # Arguments
    ///
    /// * `value` - The value to be inserted into the buffer.
    ///
    /// # Returns
    ///
    /// `Ok(())` if the element was successfully inserted.  
    /// `Err(value)` if the buffer is full.
    pub fn push(&self, value: T) -> Result<(), T> {
        loop {
            let write = self.write_index.load(Ordering::Acquire); // Load the current write index
            let read = self.read_index.load(Ordering::Acquire); // Load the current read index

            // Check if the buffer is full
            if write - read >= self.size {
                return Err(value);
            }

            let cell = &self.buffer[write % self.size]; // Find the position to write

            // Perform a Compare-And-Swap (CAS) operation on the write index
            if self
                .write_index
                .compare_exchange(
                    write,     // Expected value
                    write + 1, // New value
                    Ordering::Release,
                    Ordering::Relaxed,
                )
                .is_ok()
            {
                // CAS was successful, write the value to the cell
                unsafe {
                    let slot = &mut *cell.get();
                    *slot = Some(value); // Safely write the value
                }
                return Ok(()); // Successfully inserted the value
            }

            // If CAS fails, retry the operation
        }
    }

    /// Pops an element from the buffer.
    ///
    /// # Returns
    ///
    /// `Some(value)` if an element was successfully retrieved.  
    /// `None` if the buffer is empty.
    pub fn pop(&self) -> Option<T> {
        loop {
            let read = self.read_index.load(Ordering::Acquire); // Load the current read index
            let write = self.write_index.load(Ordering::Acquire); // Load the current write index

            if read == write {
                // Buffer is empty, nothing to read
                return None;
            }

            let cell = &self.buffer[read % self.size]; // Find the position to read

            unsafe {
                if let Some(value) = (*cell.get()).take() {
                    // If the cell contains a value, extract it

                    // Update the read index using CAS
                    if self
                        .read_index
                        .compare_exchange(read, read + 1, Ordering::Release, Ordering::Relaxed)
                        .is_ok()
                    {
                        return Some(value);
                    }
                }
            }
        }
    }

    /// Clears the buffer by resetting both read and write indices and removing all elements.
    pub fn clear(&self) {
        // Reset the read and write indices
        self.write_index.store(0, Ordering::Release);
        self.read_index.store(0, Ordering::Release);

        // Clear all elements in the buffer
        for cell in &self.buffer {
            unsafe {
                (*cell.get()).take();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*; // Импортируем RingBuffer из текущего модуля
    use std::sync::Arc;
    use tokio::task;
    use tokio::time::{sleep, Duration};

    #[test]
    fn test_push_and_pop() {
        let buffer = RingBuffer::new(3); // Буфер размером 3

        // Добавляем элементы в буфер
        assert_eq!(buffer.push(1), Ok(()));
        assert_eq!(buffer.push(2), Ok(()));
        assert_eq!(buffer.push(3), Ok(()));

        // Проверяем, что добавление четвёртого элемента вызывает переполнение
        assert_eq!(buffer.push(4), Err(4));

        // Извлекаем элементы
        assert_eq!(buffer.pop(), Some(1));
        assert_eq!(buffer.pop(), Some(2));
        assert_eq!(buffer.pop(), Some(3));

        // Проверяем, что буфер теперь пуст
        assert_eq!(buffer.pop(), None);
    }

    #[test]
    fn test_overwrite_behavior() {
        let buffer = RingBuffer::new(2); // Буфер размером 2

        // Добавляем элементы
        assert_eq!(buffer.push(1), Ok(()));
        assert_eq!(buffer.push(2), Ok(()));

        // Извлекаем один элемент
        assert_eq!(buffer.pop(), Some(1));

        // Добавляем ещё один элемент
        assert_eq!(buffer.push(3), Ok(()));

        // Проверяем, что элементы извлекаются в правильном порядке
        assert_eq!(buffer.pop(), Some(2));
        assert_eq!(buffer.pop(), Some(3));
        assert_eq!(buffer.pop(), None); // Буфер пуст
    }

    #[test]
    fn test_empty_buffer() {
        let buffer = RingBuffer::new(3); // Буфер размером 3

        // Проверяем, что пустой буфер возвращает None при pop
        assert_eq!(buffer.pop(), None);

        // Проверяем, что push работает после этого
        assert_eq!(buffer.push(1), Ok(()));
        assert_eq!(buffer.pop(), Some(1));
    }

    #[test]
    fn test_full_buffer() {
        let buffer = RingBuffer::new(3); // Буфер размером 3

        // Заполняем буфер
        assert_eq!(buffer.push(1), Ok(()));
        assert_eq!(buffer.push(2), Ok(()));
        assert_eq!(buffer.push(3), Ok(()));

        // Проверяем, что буфер переполняется
        assert_eq!(buffer.push(4), Err(4));

        // Проверяем, что данные в буфере не повреждены
        assert_eq!(buffer.pop(), Some(1));
        assert_eq!(buffer.pop(), Some(2));
        assert_eq!(buffer.pop(), Some(3));
        assert_eq!(buffer.pop(), None); // Буфер пуст
    }

    #[test]
    fn test_circular_behavior() {
        let buffer = RingBuffer::new(3); // Буфер размером 3

        // Добавляем и извлекаем элементы циклически
        assert_eq!(buffer.push(1), Ok(()));
        assert_eq!(buffer.push(2), Ok(()));
        assert_eq!(buffer.pop(), Some(1));
        assert_eq!(buffer.pop(), Some(2));
        assert_eq!(buffer.push(3), Ok(()));
        assert_eq!(buffer.push(4), Ok(()));
        assert_eq!(buffer.push(5), Ok(())); // Заполняем буфер
        assert_eq!(buffer.push(6), Err(6)); // Переполнение

        // Проверяем содержимое
        assert_eq!(buffer.pop(), Some(3));
        assert_eq!(buffer.pop(), Some(4));
        assert_eq!(buffer.pop(), Some(5));
        assert_eq!(buffer.pop(), None); // Буфер пуст
    }

    #[tokio::test]
    async fn test_async_push_pop() {
        let buffer = Arc::new(RingBuffer::new(5)); // Буфер фиксированного размера

        let buffer_writer = Arc::clone(&buffer);
        let buffer_reader = Arc::clone(&buffer);

        // Запускаем писателя (пишет в буфер)
        let writer = task::spawn(async move {
            for i in 0..10 {
                loop {
                    if buffer_writer.push(i).is_ok() {
                        break;
                    }
                    // Если буфер полон, ждём перед повторной попыткой
                    sleep(Duration::from_millis(10)).await;
                }
            }
        });

        // Запускаем читателя (чтение из буфера)
        let reader = task::spawn(async move {
            let mut results = Vec::new();
            for _ in 0..10 {
                loop {
                    if let Some(value) = buffer_reader.pop() {
                        results.push(value);
                        break;
                    }
                    // Если буфер пуст, ждём перед повторной попыткой
                    sleep(Duration::from_millis(10)).await;
                }
            }
            results
        });

        // Ждём завершения обеих задач
        writer.await.unwrap();
        let results = reader.await.unwrap();

        // Проверяем, что результаты соответствуют входным данным
        assert_eq!(results, (0..10).collect::<Vec<_>>());
    }

    #[tokio::test]
    async fn test_async_concurrent_push_and_pop() {
        let buffer = Arc::new(RingBuffer::new(10)); // Буфер размером 10

        // Создаём 3 писателя
        let writers: Vec<_> = (0..3)
            .map(|id| {
                let buffer = Arc::clone(&buffer);
                task::spawn(async move {
                    for i in 0..10 {
                        let value = id * 10 + i;
                        loop {
                            if buffer.push(value).is_ok() {
                                break;
                            }
                            // Если буфер полон, ждём перед повторной попыткой
                            sleep(Duration::from_millis(5)).await;
                        }
                    }
                })
            })
            .collect();

        // Создаём читателя
        let buffer_reader = Arc::clone(&buffer);
        let reader = task::spawn(async move {
            let mut results = Vec::new();
            for _ in 0..30 {
                loop {
                    if let Some(value) = buffer_reader.pop() {
                        results.push(value);
                        break;
                    }
                    // Если буфер пуст, ждём перед повторной попыткой
                    sleep(Duration::from_millis(5)).await;
                }
            }
            results
        });

        // Ждём завершения всех писателей
        for writer in writers {
            writer.await.unwrap();
        }

        // Ждём завершения читателя
        let results = reader.await.unwrap();

        // Проверяем, что все данные были корректно записаны и прочитаны
        let mut expected: Vec<_> = (0..3)
            .flat_map(|id| (0..10).map(move |i| id * 10 + i))
            .collect();
        expected.sort();

        let mut sorted_results = results.clone();
        sorted_results.sort();

        assert_eq!(sorted_results, expected);
    }

    #[tokio::test]
    async fn test_push_in_concurrent_env() {
        let buffer = Arc::new(RingBuffer::new(10)); // Размер буфера 10
        let barrier = Arc::new(tokio::sync::Barrier::new(5)); // Барьер для синхронизации потоков (4 писателя + 1 читатель)

        let mut handles = vec![];

        // Писатели: 4 потока, каждый пишет 5 элементов
        for i in 0..4 {
            let buffer = Arc::clone(&buffer);
            let barrier = Arc::clone(&barrier);

            handles.push(tokio::spawn(async move {
                barrier.wait().await; // Все задачи стартуют одновременно
                for j in 0..5 {
                    loop {
                        if buffer.push(i * 10 + j).is_ok() {
                            break; // Успешно записали, выходим из цикла
                        }
                        // Если буфер полон, уступаем выполнение
                        tokio::task::yield_now().await;
                    }
                }
            }));
        }

        // Читатель: 1 поток, извлекает данные из буфера
        let buffer_reader = Arc::clone(&buffer);
        let barrier_reader = Arc::clone(&barrier);

        let reader = tokio::spawn(async move {
            barrier_reader.wait().await; // Ожидаем синхронизации с писателями
            let mut results = vec![];
            for _ in 0..20 {
                loop {
                    if let Some(value) = buffer_reader.pop() {
                        results.push(value);
                        break; // Успешно извлекли значение, выходим из цикла
                    }
                    // Если буфер пуст, уступаем выполнение
                    tokio::task::yield_now().await;
                }
            }
            results // Возвращаем собранные данные
        });

        // Ожидаем завершения всех потоков
        for handle in handles {
            handle.await.unwrap(); // Убеждаемся, что все писатели завершены
        }

        // Получаем данные от читателя
        let results = reader.await.unwrap();

        // Проверяем, что данные из буфера корректны
        let mut sorted_results = results.clone();
        sorted_results.sort(); // Сортируем для предсказуемости

        let expected: Vec<_> = (0..4)
            .flat_map(|i| (0..5).map(move |j| i * 10 + j))
            .collect();
        assert_eq!(sorted_results, expected);
    }
}
