//! Zero-filled, surface-sized scratch buffers for effect layers.
//!
//! A large raster allocates one canvas-sized layer per mask, clip and
//! opacity group, but each layer usually receives paint in a small part of
//! the canvas. The global allocator recycles freed blocks and must clear
//! every byte of each one; a fresh anonymous mapping is zero by construction
//! and only the pages a layer actually writes become resident.

use std::ops::{Deref, DerefMut};

/// Layers at least this large map their own pages.
#[cfg(unix)]
const MAPPED_BYTES: usize = 1 << 20;

pub(super) struct ZeroedLayer {
    storage: Storage,
}

enum Storage {
    Heap(Vec<u8>),
    #[cfg(unix)]
    Mapped {
        pointer: std::ptr::NonNull<u8>,
        len: usize,
    },
}

impl ZeroedLayer {
    pub(super) fn new(len: usize) -> Self {
        #[cfg(unix)]
        if len >= MAPPED_BYTES {
            // SAFETY: a private anonymous mapping aliases no other memory.
            // The result is checked for failure before use.
            let pointer = unsafe {
                libc::mmap(
                    std::ptr::null_mut(),
                    len,
                    libc::PROT_READ | libc::PROT_WRITE,
                    libc::MAP_PRIVATE | libc::MAP_ANON,
                    -1,
                    0,
                )
            };
            if pointer != libc::MAP_FAILED
                && let Some(pointer) = std::ptr::NonNull::new(pointer.cast::<u8>())
            {
                return Self {
                    storage: Storage::Mapped { pointer, len },
                };
            }
        }
        Self {
            storage: Storage::Heap(vec![0; len]),
        }
    }
}

impl ZeroedLayer {
    /// Reset a reused layer without retaining pages from its earlier position.
    /// Mappings keep their frame coordinates; anonymous discarded pages read
    /// back as zero. Heap layers use the same clear as Go's offscreen cache.
    pub(super) fn clear(&mut self) {
        #[cfg(target_os = "linux")]
        if let Storage::Mapped { pointer, len } = self.storage {
            // SAFETY: the page-aligned mapping is owned exclusively by self.
            // madvise rounds its length up to the mapping's final page.
            if unsafe { libc::madvise(pointer.as_ptr().cast(), len, libc::MADV_DONTNEED) } == 0 {
                return;
            }
        }
        self.fill(0);
    }

    /// Release the memory behind `range` once its bytes are no longer
    /// needed. A mapped layer returns the whole pages inside the range to
    /// the system, which reads them back as zero; other bytes are kept.
    pub(super) fn discard(&mut self, range: std::ops::Range<usize>) {
        #[cfg(unix)]
        if let Storage::Mapped { pointer, len } = self.storage {
            // SAFETY: sysconf has no preconditions.
            let page = usize::try_from(unsafe { libc::sysconf(libc::_SC_PAGESIZE) })
                .ok()
                .filter(|&page| page > 0)
                .unwrap_or(1 << 16);
            let start = range.start.min(len).div_ceil(page) * page;
            let end = range.end.min(len) / page * page;
            if start < end {
                // SAFETY: the pages lie inside this layer's own mapping, and
                // no slice of it is borrowed while `self` is borrowed mutably.
                unsafe {
                    libc::madvise(
                        pointer.as_ptr().add(start).cast(),
                        end - start,
                        libc::MADV_DONTNEED,
                    );
                }
            }
        }
        #[cfg(not(unix))]
        let _ = range;
    }
}

impl Deref for ZeroedLayer {
    type Target = [u8];

    fn deref(&self) -> &[u8] {
        match &self.storage {
            Storage::Heap(bytes) => bytes,
            // SAFETY: the mapping is `len` readable bytes owned by `self`.
            #[cfg(unix)]
            Storage::Mapped { pointer, len } => unsafe {
                std::slice::from_raw_parts(pointer.as_ptr(), *len)
            },
        }
    }
}

impl DerefMut for ZeroedLayer {
    fn deref_mut(&mut self) -> &mut [u8] {
        match &mut self.storage {
            Storage::Heap(bytes) => bytes,
            // SAFETY: the mapping is `len` writable bytes uniquely borrowed
            // through `self`.
            #[cfg(unix)]
            Storage::Mapped { pointer, len } => unsafe {
                std::slice::from_raw_parts_mut(pointer.as_ptr(), *len)
            },
        }
    }
}

impl Drop for ZeroedLayer {
    fn drop(&mut self) {
        #[cfg(unix)]
        if let Storage::Mapped { pointer, len } = self.storage {
            // SAFETY: the mapping was created by `new` and is unmapped once.
            unsafe {
                libc::munmap(pointer.as_ptr().cast(), len);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::ZeroedLayer;

    #[test]
    fn layers_start_zeroed_and_are_writable() {
        for len in [0, 17, 3 << 20] {
            let mut layer = ZeroedLayer::new(len);
            assert_eq!(layer.len(), len);
            assert!(layer.iter().all(|&byte| byte == 0));
            if len > 0 {
                layer[len - 1] = 7;
                assert_eq!(layer[len - 1], 7);
            }
            layer.clear();
            assert!(layer.iter().all(|&byte| byte == 0));
        }
    }
}
