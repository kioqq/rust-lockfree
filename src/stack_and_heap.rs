use std::sync::Arc;
use std::thread;

pub fn stack_and_heap_vars_example() {
    let stack_var = 42; // stack variable (allocated on the stack)
    let heap_var = Box::new(42); // heap variable (allocated on the heap)

    // Box<T>, Vec<T>, Arc<T> - heap variables

    println!("stack_var: {}", stack_var);
    println!("heap_var: {}", *heap_var);
}

// #################################
// Heap и Arc<T>
// #################################

// Arc<T> позволяет нескольким потокам владеть одной кучевой переменной.
// Arc - атомарный счётчик ссылок
// Без Arc<T> код бы не скомпилировался из-за правил владения (ownership).
// Heap может разрастаться, пока есть свободная оперативная память.
// Если памяти не осталось – ядро Linux может убить процесс (OOM killer).
pub fn heap_example() {
    let shared_data = Arc::new(vec![1, 2, 3]);

    for _ in 0..3 {
        let data = Arc::clone(&shared_data);
        thread::spawn(move || {
            println!("{:?}", data);
        });
    }
}

// #################################
// Стек и переполнение стека
// #################################
// Стек - это область памяти, где хранятся локальные переменные и аргументы функций.
// Потоки не видят друг друга через стек, потому что он отдельный для каждого потока.
// Размер стека ограничен (по умолчанию 2MB на поток в Rust)
pub fn stack_example() {
    let handle = thread::spawn(|| {
        let local_variable = 42; // Это переменная в стеке потока
        println!("Локальная переменная: {}", local_variable);
    });

    handle.join().unwrap();
}

// Глубокие рекурсивные вызовы могут привести к stack overflow.
#[allow(unconditional_recursion)]
pub fn stack_overflow_example(_x: u32) {
    stack_overflow_example(_x + 1);
}

// Как изменить размер стека?
pub fn how_make_stack_bigger() {
    let builder = thread::Builder::new().stack_size(8 * 1024 * 1024); // 8MB стек

    let handle = builder
        .spawn(|| {
            println!("Используем стек 8MB!");
        })
        .unwrap();

    handle.join().unwrap();
}
