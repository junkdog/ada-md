//! Where a text's lines start, and how many lines a code block has (PAR-003). Every document's
//! whole text passes through [`line_starts`] once, and every code block's through
//! [`code_line_count`], so neither tests a byte at a time.

/// The byte offset of the start of each line.
pub fn line_starts(text: &str) -> Vec<u32> {
    let mut starts = vec![0];
    #[cfg(target_arch = "x86_64")]
    let pushed = avx2::push(text.as_bytes(), &mut starts);
    #[cfg(not(target_arch = "x86_64"))]
    let pushed = false;
    if !pushed {
        push_by_memchr(text.as_bytes(), &mut starts);
    }
    // a final newline starts no line
    if starts.len() > 1 && starts.last() == Some(&(text.len() as u32)) {
        starts.pop();
    }
    starts
}

/// Pushes the start of the line after each newline in `bytes`, where [`avx2`] can't.
fn push_by_memchr(bytes: &[u8], starts: &mut Vec<u32>) {
    starts.extend(memchr::memchr_iter(b'\n', bytes).map(|i| i as u32 + 1));
}

/// The number of lines [`super::code_lines`] yields, counted without splitting them.
pub fn code_line_count(text: &str) -> usize {
    let text = text.strip_suffix('\n').unwrap_or(text);
    if text.is_empty() {
        0
    } else {
        memchr::memchr_iter(b'\n', text.as_bytes()).count() + 1
    }
}

/// Newlines 64 bytes at a time: a mask from two compares, its set bits written four at a time
/// whether or not they're set, the length moved by the mask's count. Notes have a newline every
/// 30–80 bytes, so a chunk holds one or two and a step takes them all without a branch per
/// newline: 2–7× memchr's iterator, which returns one match a call (experiment 65).
#[cfg(target_arch = "x86_64")]
mod avx2 {
    use std::arch::x86_64::*;

    /// Positions written a step.
    const STEP: usize = 4;

    /// Pushes the start of the line after each newline in `bytes`; `false`, pushing nothing,
    /// when the CPU lacks AVX2, POPCNT or BMI1 (every CPU with AVX2 has the other two).
    pub fn push(bytes: &[u8], starts: &mut Vec<u32>) -> bool {
        let ok = is_x86_feature_detected!("avx2")
            && is_x86_feature_detected!("popcnt")
            && is_x86_feature_detected!("bmi1");
        if ok {
            // SAFETY: the CPU has every feature `push_starts` enables
            unsafe { push_starts(bytes, starts) };
        }
        ok
    }

    #[target_feature(enable = "avx2,popcnt,bmi1")]
    fn push_starts(bytes: &[u8], starts: &mut Vec<u32>) {
        let nl = _mm256_set1_epi8(b'\n' as i8);
        let (chunks, rest) = bytes.as_chunks::<64>();
        // room reserved once per 64 chunks, the most newlines they hold, and a step's spill
        for (g, group) in chunks.chunks(64).enumerate() {
            let room = group.len() * 64 + STEP;
            starts.reserve(room);
            let len = starts.len();
            let spare = &mut starts.spare_capacity_mut()[..room];
            let mut at = 0;
            for (k, chunk) in group.iter().enumerate() {
                let p = chunk.as_ptr();
                // SAFETY: both 32-byte loads are inside the chunk; neither needs alignment
                let (lo, hi) = unsafe {
                    (
                        _mm256_loadu_si256(p.cast()),
                        _mm256_loadu_si256(p.add(32).cast()),
                    )
                };
                let lo = _mm256_movemask_epi8(_mm256_cmpeq_epi8(lo, nl)) as u32;
                let hi = _mm256_movemask_epi8(_mm256_cmpeq_epi8(hi, nl)) as u32;
                let mut mask = u64::from(lo) | u64::from(hi) << 32;
                if mask == 0 {
                    continue;
                }
                let base = ((g * 64 + k) * 64) as u32 + 1;
                let n = mask.count_ones() as usize;
                let mut i = 0;
                while i < n {
                    // past the mask's last bit these write `base + 64`, within `room` and
                    // overwritten by the next chunk's or left past the length
                    for j in 0..STEP {
                        spare[at + i + j].write(base + mask.trailing_zeros());
                        mask &= mask.wrapping_sub(1);
                    }
                    i += STEP;
                }
                at += n;
            }
            // SAFETY: `spare`'s first `at` were written above
            unsafe { starts.set_len(len + at) };
        }
        let base = chunks.len() * 64;
        for (i, _) in rest.iter().enumerate().filter(|&(_, &b)| b == b'\n') {
            starts.push((base + i) as u32 + 1);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The definition, a byte at a time.
    fn starts_by_byte(text: &str) -> Vec<u32> {
        std::iter::once(0)
            .chain(
                text.bytes()
                    .enumerate()
                    .filter(|&(_, b)| b == b'\n')
                    .map(|(i, _)| i as u32 + 1),
            )
            .filter(|&s| s < text.len() as u32 || s == 0)
            .collect()
    }

    /// Texts with newlines at every density and position: none, all, each chunk boundary and
    /// group boundary, and a tail under a chunk.
    fn texts() -> Vec<String> {
        let mut texts: Vec<String> = ["", "\n", "\n\n", "a", "a\n", "\na", "a\r\nb\r\n", "ü\nü"]
            .map(String::from)
            .into();
        let mut seed = 0x5041_5230_3033_u64;
        for len in [63, 64, 65, 127, 128, 129, 4095, 4096, 4097, 4160, 9000] {
            for every in [1, 2, 3, 7, 31, 64, 65, 1000, usize::MAX] {
                let text: String = (0..len)
                    .map(|i| {
                        if every != usize::MAX && i % every == every - 1 {
                            '\n'
                        } else {
                            'x'
                        }
                    })
                    .collect();
                texts.push(text);
            }
            // newlines at random, about one in five bytes
            let text: String = (0..len)
                .map(|_| {
                    seed ^= seed << 13;
                    seed ^= seed >> 7;
                    seed ^= seed << 17;
                    if seed.is_multiple_of(5) { '\n' } else { 'y' }
                })
                .collect();
            texts.push(text);
        }
        texts
    }

    #[test]
    fn starts_match_a_byte_at_a_time() {
        for text in texts() {
            assert_eq!(line_starts(&text), starts_by_byte(&text), "{text:?}");
            // the other path too, wherever the CPU takes one
            let mut starts = vec![0];
            push_by_memchr(text.as_bytes(), &mut starts);
            let mut all = starts_by_byte(&text);
            if text.ends_with('\n') {
                all.push(text.len() as u32);
            }
            assert_eq!(starts, all, "{text:?}");
        }
    }

    #[test]
    fn line_counts_match_code_lines() {
        for text in texts() {
            assert_eq!(
                code_line_count(&text),
                super::super::code_lines(&text).count(),
                "{text:?}"
            );
        }
    }
}
