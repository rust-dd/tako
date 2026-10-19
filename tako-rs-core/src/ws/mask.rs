/// XORs `data` with a client frame mask, starting at mask offset 0.
pub(crate) fn unmask(data: &mut [u8], mask: [u8; 4]) {
  let word = u64::from_ne_bytes([
    mask[0], mask[1], mask[2], mask[3], mask[0], mask[1], mask[2], mask[3],
  ]);
  let (words, rest) = data.as_chunks_mut::<8>();
  for chunk in words {
    *chunk = (u64::from_ne_bytes(*chunk) ^ word).to_ne_bytes();
  }
  for (byte, key) in rest.iter_mut().zip(mask.iter().cycle()) {
    *byte ^= key;
  }
}

#[cfg(test)]
mod tests {
  use super::unmask;

  fn naive(data: &mut [u8], mask: [u8; 4]) {
    for (i, byte) in data.iter_mut().enumerate() {
      *byte ^= mask[i % 4];
    }
  }

  #[test]
  fn matches_the_bytewise_xor_at_every_length_and_alignment() {
    let mask = [0x37, 0xfa, 0x21, 0x3d];
    let source: Vec<u8> = (0..72u8)
      .map(|i| i.wrapping_mul(31).wrapping_add(7))
      .collect();
    for offset in 0..8 {
      for len in 0..=64 {
        let mut fast = source.clone();
        let mut slow = source.clone();
        unmask(&mut fast[offset..offset + len], mask);
        naive(&mut slow[offset..offset + len], mask);
        assert_eq!(fast, slow, "offset {offset}, len {len}");
        unmask(&mut fast[offset..offset + len], mask);
        assert_eq!(fast, source, "offset {offset}, len {len}");
      }
    }
  }
}
