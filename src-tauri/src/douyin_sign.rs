//! a_bogus port of DouyinLiveRecorder/src/ab_sign.py (MIT, Hmily).
use sm3::{Digest, Sm3};
fn rc4(bytes: &[u8], key: &[u8]) -> Vec<u8> {
    let mut state: Vec<u8> = (0..=255).collect();
    let mut j = 0usize;
    for i in 0..256 {
        j = (j + state[i] as usize + key[i % key.len()] as usize) % 256;
        state.swap(i, j);
    }
    let (mut i, mut j) = (0usize, 0usize);
    bytes
        .iter()
        .map(|b| {
            i = (i + 1) % 256;
            j = (j + state[i] as usize) % 256;
            state.swap(i, j);
            b ^ state[(state[i] as usize + state[j] as usize) % 256]
        })
        .collect()
}
fn encode(bytes: &[u8], table: &[u8]) -> String {
    let mut output = String::new();
    for index in 0..(bytes.len() * 4).div_ceil(3) {
        let start = index / 4 * 3;
        let value = ((bytes[start] as u32) << 16)
            | ((bytes.get(start + 1).copied().unwrap_or(0) as u32) << 8)
            | bytes.get(start + 2).copied().unwrap_or(0) as u32;
        output.push(table[((value >> (18 - 6 * (index % 4))) & 63) as usize] as char);
    }
    output
}
pub fn sign(query: &str, ua: &str, time_ms: u64) -> String {
    let hash = |bytes: &[u8]| Sm3::digest(bytes).to_vec();
    let params = hash(&hash(format!("{query}cus").as_bytes()));
    let suffix = hash(&hash(b"cus"));
    let ua = hash(
        encode(
            &rc4(ua.as_bytes(), &[0, 1, 14]),
            b"ckdp1h4ZKsUB80/Mfvw36XIgR25+WQAlEi7NLboqYTOPuzmFjJnryx9HVGDaStCe",
        )
        .as_bytes(),
    );
    let mut b = [0u8; 73];
    b[18] = 44;
    b[20..24].copy_from_slice(&(time_ms as u32).to_be_bytes());
    b[24] = (time_ms >> 32) as u8;
    b[25] = (time_ms >> 40) as u8;
    b[31] = 1;
    b[37] = 14;
    b[38] = params[21];
    b[39] = params[22];
    b[40] = suffix[21];
    b[41] = suffix[22];
    b[42] = ua[23];
    b[43] = ua[24];
    let end = time_ms + 100;
    b[44..48].copy_from_slice(&(end as u32).to_be_bytes());
    b[48] = 3;
    b[49] = (end >> 32) as u8;
    b[50] = (end >> 40) as u8;
    b[52..56].copy_from_slice(&110624u32.to_be_bytes());
    b[57..61].copy_from_slice(&6383u32.to_le_bytes());
    let env = b"1920|1080|1920|1040|0|30|0|0|1872|92|1920|1040|1857|92|1|24|Win32";
    b[65] = env.len() as u8;
    let checksum = [
        18, 20, 26, 30, 38, 40, 42, 21, 27, 31, 35, 39, 41, 43, 22, 28, 32, 36, 23, 29, 33, 37, 44,
        45, 46, 47, 48, 49, 50, 24, 25, 52, 53, 54, 55, 57, 58, 59, 60, 65, 66, 70, 71,
    ]
    .iter()
    .fold(0, |v, i| v ^ b[*i]);
    let mut payload: Vec<u8> = [
        18, 20, 52, 26, 30, 34, 58, 38, 40, 53, 42, 21, 27, 54, 55, 31, 35, 57, 39, 41, 43, 22, 28,
        32, 60, 36, 23, 29, 33, 37, 44, 45, 59, 46, 47, 48, 49, 50, 24, 25, 65, 66, 70, 71,
    ]
    .iter()
    .map(|i| b[*i])
    .collect();
    payload.extend_from_slice(env);
    payload.push(checksum);
    let mut bytes = Vec::new();
    for (n, options) in [(1234u16, [3, 45]), (9876, [1, 0]), (5555, [1, 5])] {
        let lo = n as u8;
        let hi = (n >> 8) as u8;
        bytes.extend([
            (lo & 170) | (options[0] & 85),
            (lo & 85) | (options[0] & 170),
            (hi & 170) | (options[1] & 85),
            (hi & 85) | (options[1] & 170),
        ]);
    }
    bytes.extend(rc4(&payload, &[121]));
    encode(
        &bytes,
        b"Dkdpgh2ZmsQB80/MfvV36XI1R45-WUAlEixNLwoqYTOPuzKFjJnry79HbGcaStCe",
    ) + "="
}

#[cfg(test)]
mod tests {
    #[test]
    fn matches_reference_at_fixed_time() {
        assert_eq!(super::sign("aid=6383&web_rid=123", "Mozilla/5.0", 1700000000000), "E7mhBmg6mEVNgf6X56KLfY3q6RF3YIoI0HViMD2fkxfLqL39HMYD9exoIBGvXKWjwG/-IeYjy4hbO3xprQAjM36UHWwEUdQ2mgWkKl5Q5I0j53iruyRDntmF4vj3SFlm5XNAEOk0y75rKb70Woqe-vIlO62-zo0/9VW=");
    }
}
