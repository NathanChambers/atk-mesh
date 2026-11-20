const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

pub fn encode(bytes: &[u8]) -> String {
    let mut output = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let a = chunk[0] as usize;
        let b = chunk.get(1).copied().unwrap_or(0) as usize;
        let c = chunk.get(2).copied().unwrap_or(0) as usize;
        output.push(ALPHABET[a >> 2] as char);
        output.push(ALPHABET[((a & 3) << 4) | (b >> 4)] as char);
        output.push(if chunk.len() > 1 {
            ALPHABET[((b & 15) << 2) | (c >> 6)] as char
        } else {
            '='
        });
        output.push(if chunk.len() > 2 {
            ALPHABET[c & 63] as char
        } else {
            '='
        });
    }
    output
}

pub fn decode(value: &str) -> Result<Vec<u8>, String> {
    if !value.len().is_multiple_of(4) {
        return Err("invalid base64 length".into());
    }

    let mut output = Vec::with_capacity(value.len() / 4 * 3);
    for (index, chunk) in value.as_bytes().chunks(4).enumerate() {
        let mut digits = [0u8; 4];
        let padding = if chunk[2] == b'=' {
            2
        } else if chunk[3] == b'=' {
            1
        } else {
            0
        };
        let invalid_padding =
            (padding > 0 && index + 1 != value.len() / 4) || (padding == 2 && chunk[3] != b'=');
        if invalid_padding {
            return Err("invalid base64 padding".into());
        }

        for i in 0..4 - padding {
            digits[i] = ALPHABET
                .iter()
                .position(|c| *c == chunk[i])
                .ok_or("invalid base64 character")? as u8;
        }
        if (padding == 2 && digits[1] & 15 != 0) || (padding == 1 && digits[2] & 3 != 0) {
            return Err("non-canonical base64 padding".into());
        }

        output.push((digits[0] << 2) | (digits[1] >> 4));
        if padding < 2 {
            output.push((digits[1] << 4) | (digits[2] >> 2));
        }
        if padding == 0 {
            output.push((digits[2] << 6) | digits[3]);
        }
    }
    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn standard_vectors_and_invalid_padding() {
        for (raw, encoded) in [
            ("", ""),
            ("f", "Zg=="),
            ("fo", "Zm8="),
            ("foo", "Zm9v"),
            ("foob", "Zm9vYg=="),
            ("fooba", "Zm9vYmE="),
            ("foobar", "Zm9vYmFy"),
        ] {
            assert_eq!(encode(raw.as_bytes()), encoded);
            assert_eq!(decode(encoded).unwrap(), raw.as_bytes());
        }
        let bytes: Vec<u8> = (0..=255).collect();
        assert_eq!(decode(&encode(&bytes)).unwrap(), bytes);
        for invalid in ["x", "====", "Zm9=", "Zg==Zg==", "Zg=a", "Z===", "Zg?="] {
            assert!(decode(invalid).is_err());
        }
    }
}
