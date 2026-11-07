// The C ABI of Caper's shared Rust logic (apps/native/mobile/src/c_abi.rs).
// Text is UTF-8; results are flat int32 arrays the caller frees with
// caper_mobile_ints_free. See src/lib.rs for each array's layout.
#ifndef CAPER_MOBILE_H
#define CAPER_MOBILE_H

#include <stdbool.h>
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
// Four values per @mention token: utf16 start, utf16 end, utf8 start, utf8 end.
CaperMobileInts caper_mobile_mention_spans(const uint8_t *utf8, size_t len);
// An edit-history comparison's highlights: the number of before spans, then
// four values per highlighted span (utf16 start, utf16 end, utf8 start, utf8
// end), before spans first.
CaperMobileInts caper_mobile_history_diff_spans(const uint8_t *before, size_t before_len, const uint8_t *after,
                                                size_t after_len);
// One composer edit (shared/messages/composer.rs). start and end are UTF-16
// offsets into next; continuing is the previous edit's pending style when this
// edit types at the caret it left, else -1. Returns the UTF-16 start, end and
// pending style, then the edited text's UTF-16 code units.
CaperMobileInts caper_mobile_composer_edit(const uint8_t *previous, size_t previous_len, const uint8_t *next,
                                           size_t next_len, int32_t start, int32_t end, int32_t continuing);
// Loads shared/emoji/catalog.json as the app ships it; false when the bytes are not a catalog.
bool caper_mobile_emoji_catalog(const uint8_t *json, size_t len);
// The `:` token with the caret at UTF-16 offset `caret`: its UTF-16 source range, then its query.
CaperMobileInts caper_mobile_emoji_token(const uint8_t *utf8, size_t len, int32_t caret);
// Positions in the loaded catalog web offers for `query`, best first.
CaperMobileInts caper_mobile_emoji_suggestions(const uint8_t *utf8, size_t len);
void caper_mobile_ints_free(CaperMobileInts ints);
// Whether a display or space name holds a character names may not.
bool caper_mobile_name_has_hidden(const uint8_t *utf8, size_t len);

#endif
