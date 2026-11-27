pub mod config;
pub mod session;
pub mod forward;
pub mod sftp;
pub mod ssh;
pub mod terminal;
pub mod terminal_input;
pub mod terminal_manager;
pub mod terminal_screen;

fn main() {
    println!("FinalShell Rust rewrite");
}

#[cfg(test)]
mod tests {
    #[test]
    fn basic_arithmetic() {
        assert_eq!(2 + 2, 4);
    }
}
