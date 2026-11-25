//! Where a text's lines start, and how many lines a code block has (PAR-003). Every document's
//! whole text passes through [`line_starts`] once (a markdown note's through [`scan`], which finds
//! what the parse looks for in the whole text in the same pass), and every code block's through
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
    no_final_start(&mut starts, text);
    starts
}

/// What [`scan`] finds in a markdown note's text.
pub struct Scan {
    /// [`line_starts`]'.
    pub starts: Vec<u32>,
    /// Where each run of two or more `=` has its second: a note's highlights (OBS-005) are
    /// looked for only on lines with one.
    pub marks: Vec<u32>,
    /// Whether a `$` is at or past the offset given: a note without one has no math, and no wiki
    /// link's target to read for it.
    pub dollar: bool,
}

/// [`line_starts`], the marks and a `$` past `from`, all in the newlines' pass (PAR-022): a
/// `memchr` for the `$` of its own cost long lines' parse 4%, and the marks tested a chunk at a
/// time, a branch on whether it held a `=`, another 6% (fences' 4–7% both); a separate pass of
/// the marks cost 15%. Each chunk's `==` pairs and `$`s are OR'd into a register for 64 chunks
/// at once, so text without either takes no branch for them, and `=` or `$` alone (code's
/// `a = b`, prices) never leaves the loop.
pub fn scan(text: &str, from: usize) -> Scan {
    let mut scan = Scan {
        starts: vec![0],
        marks: Vec::new(),
        dollar: false,
    };
    let bytes = text.as_bytes();
    #[cfg(target_arch = "x86_64")]
    let done = avx2::scan(bytes, from, &mut scan);
    #[cfg(not(target_arch = "x86_64"))]
    let done = false;
    if !done {
        push_by_memchr(bytes, &mut scan.starts);
        push_marks_by_memchr(bytes, &mut scan.marks);
        scan.dollar = bytes
            .get(from..)
            .is_some_and(|b| memchr::memchr(b'$', b).is_some());
    }
    no_final_start(&mut scan.starts, text);
    scan
}

/// A final newline starts no line.
fn no_final_start(starts: &mut Vec<u32>, text: &str) {
    if starts.len() > 1 && starts.last() == Some(&(text.len() as u32)) {
        starts.pop();
    }
}

/// Pushes where each run of two or more `=` in `bytes` has its second, where [`avx2`] can't.
fn push_marks_by_memchr(bytes: &[u8], marks: &mut Vec<u32>) {
    marks.extend(
        memchr::memmem::find_iter(bytes, b"==")
            .filter(|&p| p == 0 || bytes[p - 1] != b'=')
            .map(|p| p as u32 + 1),
    );
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
    use std::mem::MaybeUninit;

    /// Positions written a step.
    const STEP: usize = 4;

    /// Chunks a group: room for their newlines is reserved at once, and [`push_scan`] reads its
    /// registers of `==` pairs and `$`s once a group.
    const GROUP: usize = 64;

    /// Whether the CPU has every feature the kernels enable: AVX2, POPCNT and BMI1 (every CPU
    /// with AVX2 has the other two).
    fn supported() -> bool {
        is_x86_feature_detected!("avx2")
            && is_x86_feature_detected!("popcnt")
            && is_x86_feature_detected!("bmi1")
    }

    /// Pushes the start of the line after each newline in `bytes`; `false`, pushing nothing,
    /// when the CPU lacks a feature [`push_starts`] enables.
    pub fn push(bytes: &[u8], starts: &mut Vec<u32>) -> bool {
        let ok = supported();
        if ok {
            // SAFETY: the CPU has every feature `push_starts` enables
            unsafe { push_starts(bytes, starts) };
        }
        ok
    }

    /// [`super::scan`] of `bytes` into `scan`; `false`, finding nothing, when the CPU lacks a
    /// feature [`push_scan`] enables.
    pub fn scan(bytes: &[u8], from: usize, scan: &mut super::Scan) -> bool {
        let ok = supported();
        if ok {
            // SAFETY: the CPU has every feature `push_scan` enables
            scan.dollar = unsafe { push_scan(bytes, from, &mut scan.starts, &mut scan.marks) };
        }
        ok
    }

    /// Writes the start of the line after each newline in `mask`, a chunk's at `base` (the
    /// chunk's offset + 1), to `out` from `at`; how many. Through a fixed-size window, so no
    /// write is bounds-checked: on newline-dense text −6 to −14% of the pass (PAR-022).
    #[inline(always)]
    fn write_starts(out: &mut [MaybeUninit<u32>], at: usize, base: u32, mut mask: u64) -> usize {
        let n = mask.count_ones() as usize;
        let Some(out) = out[at..].first_chunk_mut::<{ 64 + STEP }>() else {
            unreachable!("room for a chunk's newlines and a step's spill");
        };
        let mut i = 0;
        while i < n {
            // past the mask's last bit these write `base + 64`, within the window and
            // overwritten by the next chunk's or left past the length
            for j in 0..STEP {
                out[i + j].write(base + mask.trailing_zeros());
                mask &= mask.wrapping_sub(1);
            }
            i += STEP;
        }
        n
    }

    #[target_feature(enable = "avx2,popcnt,bmi1")]
    fn push_starts(bytes: &[u8], starts: &mut Vec<u32>) {
        let nl = _mm256_set1_epi8(b'\n' as i8);
        let (chunks, rest) = bytes.as_chunks::<64>();
        // room reserved once a group, the most newlines its chunks hold, and a step's spill
        for (g, group) in chunks.chunks(GROUP).enumerate() {
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
                let mask = u64::from(lo) | u64::from(hi) << 32;
                if mask != 0 {
                    at += write_starts(spare, at, ((g * GROUP + k) * 64) as u32 + 1, mask);
                }
            }
            // SAFETY: `spare`'s first `at` were written above
            unsafe { starts.set_len(len + at) };
        }
        let base = chunks.len() * 64;
        for (i, _) in rest.iter().enumerate().filter(|&(_, &b)| b == b'\n') {
            starts.push((base + i) as u32 + 1);
        }
    }

    /// [`push_starts`], and the marks of the `==` pairs (the second `=` of a run's first), and
    /// whether a `$` is at or past `from`. A chunk's pairs are its `=`s beside the `=`s of the
    /// 64 bytes one on, both read by loads: the text's last byte is held out of the chunks, so
    /// the one past a chunk is always the text's. A group whose register holds a pair is read
    /// again for its marks ([`pair_marks`]); one whose register holds a `$` is searched for it
    /// past `from`, until one is found.
    #[target_feature(enable = "avx2,popcnt,bmi1")]
    fn push_scan(bytes: &[u8], from: usize, starts: &mut Vec<u32>, marks: &mut Vec<u32>) -> bool {
        let nl = _mm256_set1_epi8(b'\n' as i8);
        let eq = _mm256_set1_epi8(b'=' as i8);
        let dl = _mm256_set1_epi8(b'$' as i8);
        let mut dollar = false;
        let (chunks, _) = bytes[..bytes.len().saturating_sub(1)].as_chunks::<64>();
        for (g, group) in chunks.chunks(GROUP).enumerate() {
            let room = group.len() * 64 + STEP;
            starts.reserve(room);
            let len = starts.len();
            let spare = &mut starts.spare_capacity_mut()[..room];
            let mut at = 0;
            let mut pairs = _mm256_setzero_si256();
            let mut dollars = _mm256_setzero_si256();
            for (k, chunk) in group.iter().enumerate() {
                let p = chunk.as_ptr();
                // SAFETY: the loads at `p` and `p + 32` are inside the chunk, those at `p + 1`
                // and `p + 33` reach `p + 64`, the next chunk's first byte or the text's last
                // (held out of the chunks); none needs alignment
                let (lo, hi, lo1, hi1) = unsafe {
                    (
                        _mm256_loadu_si256(p.cast()),
                        _mm256_loadu_si256(p.add(32).cast()),
                        _mm256_loadu_si256(p.add(1).cast()),
                        _mm256_loadu_si256(p.add(33).cast()),
                    )
                };
                let pl = _mm256_and_si256(_mm256_cmpeq_epi8(lo, eq), _mm256_cmpeq_epi8(lo1, eq));
                let ph = _mm256_and_si256(_mm256_cmpeq_epi8(hi, eq), _mm256_cmpeq_epi8(hi1, eq));
                pairs = _mm256_or_si256(pairs, _mm256_or_si256(pl, ph));
                let (dlo, dhi) = (_mm256_cmpeq_epi8(lo, dl), _mm256_cmpeq_epi8(hi, dl));
                dollars = _mm256_or_si256(dollars, _mm256_or_si256(dlo, dhi));
                let lo = _mm256_movemask_epi8(_mm256_cmpeq_epi8(lo, nl)) as u32;
                let hi = _mm256_movemask_epi8(_mm256_cmpeq_epi8(hi, nl)) as u32;
                let mask = u64::from(lo) | u64::from(hi) << 32;
                if mask != 0 {
                    at += write_starts(spare, at, ((g * GROUP + k) * 64) as u32 + 1, mask);
                }
            }
            // SAFETY: `spare`'s first `at` were written above
            unsafe { starts.set_len(len + at) };
            if _mm256_testz_si256(pairs, pairs) == 0 {
                pair_marks(bytes, g * GROUP, group.len(), marks);
            }
            let end = (g * GROUP + group.len()) * 64;
            if !dollar && end > from && _mm256_testz_si256(dollars, dollars) == 0 {
                dollar = memchr::memchr(b'$', &bytes[(g * GROUP * 64).max(from)..end]).is_some();
            }
        }
        // the rest, and the text's last byte, a byte at a time
        let base = chunks.len() * 64;
        for (i, _) in bytes[base..]
            .iter()
            .enumerate()
            .filter(|&(_, &b)| b == b'\n')
        {
            starts.push((base + i) as u32 + 1);
        }
        for i in base..bytes.len().saturating_sub(1) {
            if bytes[i] == b'=' && bytes[i + 1] == b'=' && (i == 0 || bytes[i - 1] != b'=') {
                marks.push(i as u32 + 1);
            }
        }
        dollar
            || bytes
                .get(base.max(from)..)
                .is_some_and(|b| memchr::memchr(b'$', b).is_some())
    }

    /// Pushes the marks of the `==` pairs whose first `=` is in the `n` chunks from chunk
    /// `first`, none holding the text's last byte: each run's first pair's second `=`. Out of
    /// line: most groups hold none.
    #[cold]
    #[inline(never)]
    #[target_feature(enable = "avx2,popcnt,bmi1")]
    fn pair_marks(bytes: &[u8], first: usize, n: usize, marks: &mut Vec<u32>) {
        let eq = _mm256_set1_epi8(b'=' as i8);
        for c in first..first + n {
            let base = c * 64;
            let p = bytes[base..base + 65].as_ptr();
            // SAFETY: all four loads are inside the 65 bytes just indexed; none needs alignment
            let (lo, hi, lo1, hi1) = unsafe {
                (
                    _mm256_loadu_si256(p.cast()),
                    _mm256_loadu_si256(p.add(32).cast()),
                    _mm256_loadu_si256(p.add(1).cast()),
                    _mm256_loadu_si256(p.add(33).cast()),
                )
            };
            let pl = _mm256_and_si256(_mm256_cmpeq_epi8(lo, eq), _mm256_cmpeq_epi8(lo1, eq));
            let ph = _mm256_and_si256(_mm256_cmpeq_epi8(hi, eq), _mm256_cmpeq_epi8(hi1, eq));
            let mut m = u64::from(_mm256_movemask_epi8(pl) as u32)
                | u64::from(_mm256_movemask_epi8(ph) as u32) << 32;
            // a pair after a `=` is a run's later pair
            m &= !(m << 1);
            if m & 1 != 0 && base > 0 && bytes[base - 1] == b'=' {
                m &= !1;
            }
            while m != 0 {
                marks.push((base + m.trailing_zeros() as usize) as u32 + 1);
                m &= m - 1;
            }
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

    /// Where each run of two `=` or more has its second, a byte at a time.
    fn marks_by_byte(text: &str) -> Vec<u32> {
        let b = text.as_bytes();
        (1..b.len())
            .filter(|&q| b[q] == b'=' && b[q - 1] == b'=' && (q < 2 || b[q - 2] != b'='))
            .map(|q| q as u32)
            .collect()
    }

    #[test]
    fn marks_match_a_byte_at_a_time() {
        let mut texts = texts();
        let mut seed = 0x4f42_5330_3035_u64;
        for len in [
            1, 2, 3, 62, 63, 64, 65, 66, 127, 128, 129, 4095, 4096, 4097, 4160, 9000,
        ] {
            // `=` runs of every length across every chunk boundary, then `=` at random
            for every in [2, 3, 5, 61, 64, 67] {
                texts.push(
                    (0..len)
                        .map(|i| if i % every == 0 { 'x' } else { '=' })
                        .collect(),
                );
            }
            for density in [2, 3, 9] {
                texts.push(
                    (0..len)
                        .map(|_| {
                            seed ^= seed << 13;
                            seed ^= seed >> 7;
                            seed ^= seed << 17;
                            match seed % density {
                                0 => '\n',
                                1 => 'y',
                                _ => '=',
                            }
                        })
                        .collect(),
                );
            }
        }
        for text in texts {
            let Scan { starts, marks, .. } = scan(&text, 0);
            assert_eq!(starts, starts_by_byte(&text), "{text:?}");
            assert_eq!(marks, marks_by_byte(&text), "{text:?}");
            let mut by_memchr = Vec::new();
            push_marks_by_memchr(text.as_bytes(), &mut by_memchr);
            assert_eq!(by_memchr, marks_by_byte(&text), "{text:?}");
        }
    }

    #[test]
    fn a_dollar_is_found_past_from_as_memchr_finds_it() {
        let mut texts = texts();
        for len in [
            1, 2, 63, 64, 65, 66, 128, 129, 4095, 4096, 4097, 4161, 8192, 8193, 9000,
        ] {
            // one `$` at each boundary, and none
            for at in [0, 1, 62, 63, 64, 65, 4095, 4096, 4097, 8191, 8192, len - 1] {
                if at < len {
                    let mut text = "x".repeat(len);
                    text.replace_range(at..=at, "$");
                    texts.push(text);
                }
            }
        }
        for text in texts {
            let len = text.len();
            for from in [
                0,
                1,
                63,
                64,
                65,
                4095,
                4096,
                4097,
                8192,
                len.saturating_sub(1),
                len,
                len + 1,
            ] {
                let found = text
                    .as_bytes()
                    .get(from..)
                    .is_some_and(|b| b.contains(&b'$'));
                assert_eq!(scan(&text, from).dollar, found, "from {from} in {text:?}");
            }
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
