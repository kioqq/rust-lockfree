use std::cell::UnsafeCell;
use std::sync::atomic::{AtomicUsize, Ordering};

/// Lock-free кольцевой буфер с фиксированной ёмкостью.
/// Эта структура потокобезопасна и может использоваться в многопоточной среде.
pub struct RingBuffer<T> {
    buffer: Vec<UnsafeCell<Option<T>>>, // Внутренний массив для хранения данных
    size: usize,                        // Фиксированная ёмкость буфера
    write_index: AtomicUsize,           // Указатель на запись (write head)
    read_index: AtomicUsize,            // Указатель на чтение (read head)
}

// Объявляем буфер потокобезопасным, так как UnsafeCell по умолчанию не является Sync.
unsafe impl<T> Sync for RingBuffer<T> {}

impl<T> RingBuffer<T> {
    /// Создаёт новый `RingBuffer` заданного размера.
    ///
    /// # Аргументы
    ///
    /// * `size` - Максимальное количество элементов, которое может хранить буфер.
    ///
    /// # Возвращает
    ///
    /// Новый экземпляр `RingBuffer`.
    pub fn new(size: usize) -> Self {
        let mut buffer = Vec::with_capacity(size);
        for _ in 0..size {
            buffer.push(UnsafeCell::new(None)); // Заполняем буфер пустыми ячейками
        }

        RingBuffer {
            buffer,
            size,
            write_index: AtomicUsize::new(0), // Начальный индекс записи - 0
            read_index: AtomicUsize::new(0),  // Начальный индекс чтения - 0
        }
    }

    /// Добавляет элемент в буфер.
    ///
    /// # Аргументы
    ///
    /// * `value` - Значение, которое нужно вставить в буфер.
    ///
    /// # Возвращает
    ///
    /// `Ok(())`, если элемент успешно добавлен.  
    /// `Err(value)`, если буфер заполнен.
    pub fn push(&self, value: T) -> Result<(), T> {
        loop {
            let write = self.write_index.load(Ordering::Acquire); // Загружаем текущий индекс записи
            let read = self.read_index.load(Ordering::Acquire); // Загружаем текущий индекс чтения

            // Проверяем, заполнен ли буфер
            if write - read >= self.size {
                return Err(value); // Буфер полон, возврат ошибки
            }

            let cell = &self.buffer[write % self.size]; // Определяем ячейку для записи

            // Пробуем атомарно обновить индекс записи с помощью CAS (Compare-And-Swap)
            if self
                .write_index
                .compare_exchange_weak(
                    write,     // Ожидаемое значение
                    write + 1, // Новое значение (сдвигаем указатель вперёд)
                    Ordering::Release,
                    Ordering::Relaxed,
                )
                .is_ok()
            {
                // Если CAS успешно сработал, записываем значение в ячейку
                unsafe {
                    let slot = &mut *cell.get();
                    *slot = Some(value); // Безопасная запись данных
                }
                return Ok(()); // Операция записи завершена успешно
            }

            // Если CAS не сработал (другой поток изменил индекс), повторяем попытку
        }
    }

    /// Извлекает элемент из буфера.
    ///
    /// # Возвращает
    ///
    /// `Some(value)`, если элемент успешно прочитан.  
    /// `None`, если буфер пуст.
    pub fn pop(&self) -> Option<T> {
        loop {
            let read = self.read_index.load(Ordering::Acquire); // Загружаем текущий индекс чтения
            let write = self.write_index.load(Ordering::Acquire); // Загружаем текущий индекс записи

            if read == write {
                // Если индексы равны, значит, буфер пуст
                return None;
            }

            let cell = &self.buffer[read % self.size]; // Определяем ячейку для чтения

            unsafe {
                if let Some(value) = (*cell.get()).take() {
                    // Если в ячейке есть данные, извлекаем их

                    // Пробуем обновить индекс чтения атомарно с помощью CAS
                    if self
                        .read_index
                        .compare_exchange_weak(read, read + 1, Ordering::Release, Ordering::Relaxed)
                        .is_ok()
                    {
                        return Some(value); // Успешно прочитали и обновили индекс
                    }
                }
            }

            // Если CAS не сработал, пробуем снова
        }
    }

    /// Очищает буфер, сбрасывая индексы чтения и записи.
    pub fn clear(&self) {
        // Устанавливаем индексы чтения и записи в начальное состояние
        self.write_index.store(0, Ordering::Release);
        self.read_index.store(0, Ordering::Release);

        // Удаляем все элементы из буфера
        for cell in &self.buffer {
            unsafe {
                (*cell.get()).take(); // Обнуляем содержимое каждой ячейки
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
