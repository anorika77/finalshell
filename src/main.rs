pub mod config;
pub mod forward;
pub mod sftp;
pub mod ssh;
pub mod terminal;

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
