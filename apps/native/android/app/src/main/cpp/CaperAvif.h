#pragma once

#include <stdint.h>

#include "avif/avif.h"

#ifdef __cplusplus
extern "C" {
#endif

// Encodes 8-bit RGBA rows (Android's ARGB_8888 memory order) as a 4:2:0 AVIF still. Alpha is
// written only when some pixel is not opaque. On failure `output` is left empty.
avifResult caper_avif_encode(const uint8_t *rgba, uint32_t width, uint32_t height, uint32_t stride, int premultiplied,
                             int colorPrimaries, int quality, int speed, int threads, avifRWData *output);

#ifdef __cplusplus
}
#endif
