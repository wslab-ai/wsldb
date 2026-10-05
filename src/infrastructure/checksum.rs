const CRC32C_POLYNOMIAL: u32 = 0x82F6_3B78;
const CRC32C_TABLES: [[u32; 256]; 8] = build_crc32c_tables();

// Table zero consumes one byte; every later table advances that result by one
// additional zero byte. Combining all eight entries consumes one 64-bit slice.
const fn build_crc32c_tables() -> [[u32; 256]; 8] {
    let mut tables = [[0_u32; 256]; 8];
    let mut index = 0;
    while index < tables[0].len() {
        let mut crc = index as u32;
        let mut bit = 0;
        while bit < 8 {
            let mask = 0_u32.wrapping_sub(crc & 1);
            crc = (crc >> 1) ^ (CRC32C_POLYNOMIAL & mask);
            bit += 1;
        }
        tables[0][index] = crc;
        index += 1;
    }
    let mut slice = 1;
    while slice < tables.len() {
        index = 0;
        while index < tables[slice].len() {
            let crc = tables[slice - 1][index];
            tables[slice][index] = tables[0][(crc & 0xff) as usize] ^ (crc >> 8);
            index += 1;
        }
        slice += 1;
    }
    tables
}

pub(crate) fn crc32c(bytes: &[u8]) -> u32 {
    let mut crc = Crc32c::new();
    crc.update(bytes);
    crc.finish()
}

pub(crate) struct Crc32c {
    state: u32,
}

impl Crc32c {
    pub(crate) const fn new() -> Self {
        Self { state: u32::MAX }
    }

    pub(crate) fn update(&mut self, bytes: &[u8]) {
        let mut crc = self.state;
        let mut chunks = bytes.chunks_exact(8);
        for chunk in &mut chunks {
            let mut low = [0_u8; 4];
            low.copy_from_slice(&chunk[..4]);
            let low = crc ^ u32::from_le_bytes(low);
            crc = CRC32C_TABLES[7][(low & 0xff) as usize]
                ^ CRC32C_TABLES[6][((low >> 8) & 0xff) as usize]
                ^ CRC32C_TABLES[5][((low >> 16) & 0xff) as usize]
                ^ CRC32C_TABLES[4][(low >> 24) as usize]
                ^ CRC32C_TABLES[3][usize::from(chunk[4])]
                ^ CRC32C_TABLES[2][usize::from(chunk[5])]
                ^ CRC32C_TABLES[1][usize::from(chunk[6])]
                ^ CRC32C_TABLES[0][usize::from(chunk[7])];
        }
        for byte in chunks.remainder() {
            let index = ((crc ^ u32::from(*byte)) & 0xff) as usize;
            crc = CRC32C_TABLES[0][index] ^ (crc >> 8);
        }
        self.state = crc;
    }

    pub(crate) const fn finish(self) -> u32 {
        !self.state
    }
}

/// Returns the CRC32C of `left || right` from the finalized CRCs of each side.
///
/// `right_len` is measured in bytes. The operation applies the zero-byte
/// operator with exponentiation by squaring, then folds in `right_crc`.
pub(crate) fn crc32c_combine(mut left_crc: u32, right_crc: u32, mut right_len: usize) -> u32 {
    if right_len == 0 {
        return left_crc;
    }

    let mut odd = [0_u32; 32];
    odd[0] = CRC32C_POLYNOMIAL;
    let mut row = 1_u32;
    for entry in &mut odd[1..] {
        *entry = row;
        row <<= 1;
    }
    let mut even = square(&odd);
    odd = square(&even);

    loop {
        even = square(&odd);
        if right_len & 1 != 0 {
            left_crc = matrix_times(&even, left_crc);
        }
        right_len >>= 1;
        if right_len == 0 {
            break;
        }
        odd = square(&even);
        if right_len & 1 != 0 {
            left_crc = matrix_times(&odd, left_crc);
        }
        right_len >>= 1;
        if right_len == 0 {
            break;
        }
    }
    left_crc ^ right_crc
}

fn square(matrix: &[u32; 32]) -> [u32; 32] {
    let mut squared = [0_u32; 32];
    for (index, entry) in squared.iter_mut().enumerate() {
        *entry = matrix_times(matrix, matrix[index]);
    }
    squared
}

fn matrix_times(matrix: &[u32; 32], mut vector: u32) -> u32 {
    let mut sum = 0_u32;
    let mut index = 0;
    while vector != 0 {
        if vector & 1 != 0 {
            sum ^= matrix[index];
        }
        vector >>= 1;
        index += 1;
    }
    sum
}

#[cfg(test)]
mod tests {
    use super::{Crc32c, crc32c, crc32c_combine};

    const POLYNOMIAL: u32 = 0x82F6_3B78;

    fn bitwise_crc32c(bytes: &[u8]) -> u32 {
        let mut crc = u32::MAX;
        for byte in bytes {
            crc ^= u32::from(*byte);
            for _ in 0..8 {
                let mask = 0_u32.wrapping_sub(crc & 1);
                crc = (crc >> 1) ^ (POLYNOMIAL & mask);
            }
        }
        !crc
    }

    #[test]
    fn matches_standard_check_value() {
        assert_eq!(crc32c(b"123456789"), 0xE306_9283);
    }

    #[test]
    fn slicing_matches_bitwise_reference_across_lengths() {
        let bytes = (0_u16..=255).flat_map(u16::to_le_bytes).collect::<Vec<_>>();
        for len in 0..=bytes.len() {
            assert_eq!(crc32c(&bytes[..len]), bitwise_crc32c(&bytes[..len]));
        }
    }

    #[test]
    fn slicing_matches_bitwise_reference_for_varied_content_and_offsets() {
        let mut state = 0xA076_1D64_78BD_642F_u64;
        let mut bytes = vec![0_u8; 2_065];
        for byte in &mut bytes {
            state ^= state >> 12;
            state ^= state << 25;
            state ^= state >> 27;
            *byte = state.wrapping_mul(0x2545_F491_4F6C_DD1D) as u8;
        }

        for offset in 0..16 {
            for len in 0..=2_049 {
                let input = &bytes[offset..offset + len];
                assert_eq!(crc32c(input), bitwise_crc32c(input));
            }
        }
    }

    #[test]
    fn combined_crc_matches_every_deterministic_split() {
        let mut state = 0xE703_7ED1_A0B4_28DB_u64;
        let mut bytes = vec![0_u8; 8_193];
        for byte in &mut bytes {
            state ^= state >> 7;
            state ^= state << 9;
            state ^= state >> 13;
            *byte = state as u8;
        }

        for split in 0..=bytes.len() {
            assert_eq!(
                crc32c_combine(
                    crc32c(&bytes[..split]),
                    crc32c(&bytes[split..]),
                    bytes.len() - split,
                ),
                crc32c(&bytes),
                "split {split}",
            );
        }
    }

    #[test]
    fn incremental_crc_matches_every_deterministic_split() {
        let bytes = (0_u16..=255).flat_map(u16::to_le_bytes).collect::<Vec<_>>();
        for split in 0..=bytes.len() {
            let mut crc = Crc32c::new();
            crc.update(&bytes[..split]);
            crc.update(&bytes[split..]);
            assert_eq!(crc.finish(), crc32c(&bytes), "split {split}");
        }
    }
}
