pub const GREETING: &str = "Hello from {{crate_name}}!";

fn main() {
    println!("{GREETING}");
}

#[cfg(test)]
mod tests {
    use super::GREETING;

    #[test]
    fn greeting_names_the_crate() {
        assert_eq!(GREETING, "Hello from {{crate_name}}!");
    }
}
