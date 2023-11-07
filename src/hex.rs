pub fn hex(str: &[u8]) -> String {
    str.iter().map(|c| format!("{:02x}", c)).collect::<String>()
}

pub fn from_hex(str: &str) -> Vec<u8> {
    (0..str.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&str[i..i + 2], 16).unwrap())
        .collect()
}
