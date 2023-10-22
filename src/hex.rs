pub fn hex(str: &[u8]) -> String {
    str.iter().map(|c| format!("{:02x}", c)).collect::<String>()
}
