#[macro_export]
macro_rules! ruknet_debug {
    ($($arg:tt)*) => {
        if cfg!(feature = "debug") {
            println!("[DEBUG] {}", format!($($arg)*));
        }
    }
}
