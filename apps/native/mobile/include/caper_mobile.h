// The C ABI of Caper's shared Rust logic (apps/native/mobile/src/c_abi.rs).
// Text is UTF-8; results are flat int32 arrays the caller frees with
// caper_mobile_ints_free. See src/lib.rs for each array's layout.
#ifndef CAPER_MOBILE_H
#define CAPER_MOBILE_H

#include <stddef.h>
#include <stdint.h>

typedef struct {
  int32_t *data;
  size_t len;
} CaperMobileInts;

// Five values per visible span: utf16 start, utf16 end, utf8 start, utf8 end,
// style bits (1 bold, 2 italic).
CaperMobileInts caper_mobile_format_spans(const uint8_t *utf8, size_t len);
// Five values per link: utf16 start, utf16 end, utf8 start, utf8 end, scheme
// (8 https://, 7 http://, 0 www.).
CaperMobileInts caper_mobile_link_spans(const uint8_t *utf8, size_t len);
void caper_mobile_ints_free(CaperMobileInts ints);

#endif
