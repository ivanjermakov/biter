use core::fmt;
use std::{collections::BTreeMap, vec};

#[derive(Clone, PartialEq, Eq, Hash)]
pub enum BencodeValue {
    String(Vec<u8>),
    Int(i64),
    List(Vec<BencodeValue>),
    Dict(BTreeMap<String, BencodeValue>),
}

impl fmt::Debug for BencodeValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> Result<(), fmt::Error> {
        match self {
            BencodeValue::String(s) => match String::from_utf8(s.clone()) {
                Ok(str) => str.fmt(f),
                _ => f.write_str("<non-utf>"),
            },
            BencodeValue::Int(i) => i.fmt(f),
            BencodeValue::List(l) => f.debug_list().entries(l).finish(),
            BencodeValue::Dict(m) => f.debug_map().entries(m).finish(),
        }
    }
}

pub fn parse_bencoded(bencoded: Vec<u8>) -> (Option<BencodeValue>, Vec<u8>) {
    let next = bencoded.first().unwrap();
    match *next as char {
        c if c.is_ascii_digit() => parse_string(bencoded),
        'i' => parse_int(bencoded),
        'l' => parse_list(bencoded),
        'd' => parse_dict(bencoded),
        _ => {
            eprintln!("unexpected character `{}`", *next as char);
            (None, bencoded)
        }
    }
}

/// Format: <string length encoded in base ten ASCII>:<string data>
pub fn parse_string(bencoded: Vec<u8>) -> (Option<BencodeValue>, Vec<u8>) {
    let mut i = 0;

    let size_chars = bencoded
        .iter()
        .take_while(|c| (**c as char).is_ascii_digit())
        .map(|d| *d as char)
        .collect::<String>();
    let size = match size_chars.parse::<i64>() {
        Ok(n) => n,
        _ => return (None, bencoded),
    };
    i += size_chars.len();

    if bencoded.get(i).filter(|c| (**c as char) == ':').is_none() {
        return (None, bencoded);
    }
    i += 1;

    let str: Vec<u8> = bencoded
        .iter()
        .skip(size_chars.len() + 1)
        .take(size as usize)
        .cloned()
        .collect();
    i += str.len();

    (
        Some(BencodeValue::String(str)),
        bencoded.iter().skip(i).cloned().collect(),
    )
}

/// Format: i<integer encoded in base ten ASCII>e
pub fn parse_int(bencoded: Vec<u8>) -> (Option<BencodeValue>, Vec<u8>) {
    let mut i = 0;

    if bencoded.get(i).filter(|c| (**c as char) == 'i').is_none() {
        return (None, bencoded);
    }
    i += 1;

    let int_chars = bencoded
        .iter()
        .skip(i)
        .map(|c| *c as char)
        .take_while(|c| c.is_ascii_digit() || *c == '-')
        .collect::<String>();
    let int = match int_chars.parse::<i64>() {
        Ok(int) => int,
        _ => return (None, bencoded),
    };
    i += int_chars.len();

    if bencoded.get(i).filter(|c| (**c as char) == 'e').is_none() {
        return (None, bencoded);
    }
    i += 1;

    (
        Some(BencodeValue::Int(int)),
        bencoded.iter().skip(i).cloned().collect(),
    )
}

/// Format: l<bencoded values>e
pub fn parse_list(bencoded: Vec<u8>) -> (Option<BencodeValue>, Vec<u8>) {
    let mut i = 0;
    let mut items = vec![];

    if bencoded.get(i).filter(|c| (**c as char) == 'l').is_none() {
        return (None, bencoded);
    }
    i += 1;

    while bencoded.get(i).is_some() && bencoded.get(i).filter(|c| (**c as char) == 'e').is_none() {
        if let (Some(item), left) = parse_bencoded(bencoded.iter().skip(i).cloned().collect()) {
            items.push(item);
            i = bencoded.len() - left.len()
        } else {
            break;
        }
    }

    if bencoded.get(i).filter(|c| (**c as char) == 'e').is_none() {
        return (None, bencoded);
    }
    i += 1;

    (
        Some(BencodeValue::List(items)),
        bencoded.iter().skip(i).cloned().collect(),
    )
}

/// Format: d<bencoded string><bencoded element>e
pub fn parse_dict(bencoded: Vec<u8>) -> (Option<BencodeValue>, Vec<u8>) {
    let mut i = 0;
    let mut map: BTreeMap<String, BencodeValue> = BTreeMap::new();

    if bencoded.get(i).filter(|c| (**c as char) == 'd').is_none() {
        return (None, bencoded);
    }
    i += 1;

    while bencoded.get(i).is_some() && bencoded.get(i).filter(|c| (**c as char) == 'e').is_none() {
        let key = if let (Some(item), left) =
            parse_bencoded(bencoded.iter().skip(i).cloned().collect())
        {
            i = bencoded.len() - left.len();
            match item {
                BencodeValue::String(s) => match String::from_utf8(s) {
                    Ok(s) => s,
                    _ => return (None, bencoded),
                },
                _ => return (None, bencoded),
            }
        } else {
            return (None, bencoded);
        };
        let value = if let (Some(item), left) =
            parse_bencoded(bencoded.iter().skip(i).cloned().collect())
        {
            i = bencoded.len() - left.len();
            item
        } else {
            return (None, bencoded);
        };
        map.insert(key, value);
    }

    if bencoded.get(i).filter(|c| (**c as char) == 'e').is_none() {
        return (None, bencoded);
    }
    i += 1;

    (
        Some(BencodeValue::Dict(map)),
        bencoded.iter().skip(i).cloned().collect(),
    )
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn should_parse_string() {
        let (str, left) = parse_bencoded(String::into_bytes("5:hello".into()));
        assert_eq!(
            str,
            Some(BencodeValue::String(String::into_bytes("hello".into())))
        );
        assert!(left.is_empty());
    }

    #[test]
    fn should_parse_int() {
        let (str, left) = parse_bencoded(String::into_bytes("i42e".into()));
        assert_eq!(str, Some(BencodeValue::Int(42)));
        assert!(left.is_empty());
    }

    #[test]
    fn should_parse_negative_int() {
        let (str, left) = parse_bencoded(String::into_bytes("i-42e".into()));
        assert_eq!(str, Some(BencodeValue::Int(-42)));
        assert!(left.is_empty());
    }

    #[test]
    fn should_parse_list() {
        let (str, left) = parse_bencoded(String::into_bytes("l4:spam4:eggse".into()));
        assert_eq!(
            str,
            Some(BencodeValue::List(vec!(
                BencodeValue::String(String::into_bytes("spam".into())),
                BencodeValue::String(String::into_bytes("eggs".into()))
            )))
        );
        assert!(left.is_empty());
    }

    #[test]
    fn should_parse_dict() {
        let (str, left) = parse_bencoded(String::into_bytes("d3:cow3:moo4:spam4:eggse".into()));
        assert_eq!(
            str,
            Some(BencodeValue::Dict(
                [
                    (
                        "cow".into(),
                        BencodeValue::String(String::into_bytes("moo".into()))
                    ),
                    (
                        "spam".into(),
                        BencodeValue::String(String::into_bytes("eggs".into()))
                    )
                ]
                .into_iter()
                .collect()
            ))
        );
        assert!(left.is_empty());
    }
}
