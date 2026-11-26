pub mod config;
pub mod ssh;
pub mod sftp;

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
