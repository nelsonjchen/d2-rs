// SPDX-License-Identifier: MPL-2.0
// libmimalloc-sys does not expose mi_option_purge_delay in its Rust bindings.
// Use the header from the linked allocator so its enum and ABI stay in sync.
#include <mimalloc.h>

void d2_rs_allocator_collect(void) {
    mi_collect(true);
}

void d2_rs_allocator_set_purge_delay(void) {
    mi_option_set(mi_option_purge_delay, 2);
}
