use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Instant;

// Lock-free алгоритмы позволяют избежать блокировок, что улучшает производительность.
// Однако они сложны в реализации и могут быть менее безопасными.
pub fn lockfree_vs_mutex() {
    let num_threads = 4;
    let iterations = 1_000_000;

    let counter_mutex = Arc::new(Mutex::new(0));
    let counter_atomic = Arc::new(AtomicUsize::new(0));

    // Тест Mutex
    let start = Instant::now();
    let mut handles = vec![];
    for _ in 0..num_threads {
        let counter = Arc::clone(&counter_mutex);
        let handle = thread::spawn(move || {
            for _ in 0..iterations {
                let mut num = counter.lock().unwrap();
                *num += 1;
            }
        });
        handles.push(handle);
    }
    for handle in handles {
        handle.join().unwrap();
    }
    let duration_mutex = start.elapsed();

    // Тест Lock-Free (AtomicUsize)
    let start = Instant::now();
    let mut handles = vec![];
    for _ in 0..num_threads {
        let counter = Arc::clone(&counter_atomic);
        let handle = thread::spawn(move || {
            for _ in 0..iterations {
                counter.fetch_add(1, Ordering::Relaxed);
            }
        });
        handles.push(handle);
    }
    for handle in handles {
        handle.join().unwrap();
    }
    let duration_atomic = start.elapsed();

    println!("\n########################################################");
    println!("Mutex: {:?}", duration_mutex);
    println!("Lock-Free: {:?}", duration_atomic);
    println!("########################################################");

    // Вывод результатов
    let counter_mutex_value = counter_mutex.lock().unwrap();
    let counter_atomic_value = counter_atomic.load(Ordering::Relaxed);

    println!("\nCounter Mutex: {:?}", counter_mutex_value);
    println!("Counter Atomic: {:?}", counter_atomic_value);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_lockfree_vs_mutex() {
        lockfree_vs_mutex();
    }
}
