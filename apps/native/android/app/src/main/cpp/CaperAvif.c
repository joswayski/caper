// AVIF photo encoder for AttachmentPreparer (docs/media.md "Client compression and previews"):
// libavif 1.4.2 over libaom 3.15.1, 8-bit 4:2:0, full range. `quality` is libavif's quality
// scale (as `avifenc -q`), passed straight through so `avifQuality` means the same on every
// client. Built by prepare-avif.sh + CMakeLists.txt.
#include "CaperAvif.h"

#ifndef CAPER_AVIF_TUNE
#define CAPER_AVIF_TUNE "ssim"
#endif

avifResult caper_avif_encode(const uint8_t *rgba, uint32_t width, uint32_t height, uint32_t stride, int premultiplied,
                             int colorPrimaries, int quality, int speed, int threads, avifRWData *output) {
    if (!rgba || width == 0 || height == 0 || stride < width * 4 || quality < 0 || quality > 100) return AVIF_RESULT_INVALID_ARGUMENT;
    // Opaque photos (all JPEG/HEIC) carry no alpha item at all.
    int opaque = 1;
    for (uint32_t y = 0; y < height && opaque; ++y) {
        const uint8_t *row = rgba + (size_t)y * stride;
        for (uint32_t x = 0; x < width; ++x) {
            if (row[x * 4 + 3] != 255) { opaque = 0; break; }
        }
    }
    avifImage *image = avifImageCreate(width, height, 8, AVIF_PIXEL_FORMAT_YUV420);
    if (!image) return AVIF_RESULT_OUT_OF_MEMORY;
    // nclx instead of an ICC profile: sRGB (1) or Display P3 (12) primaries, sRGB transfer, BT.601.
    image->colorPrimaries = (avifColorPrimaries)colorPrimaries;
    image->transferCharacteristics = AVIF_TRANSFER_CHARACTERISTICS_SRGB;
    image->matrixCoefficients = AVIF_MATRIX_COEFFICIENTS_BT601;
    image->yuvRange = AVIF_RANGE_FULL;

    avifRGBImage rgb;
    avifRGBImageSetDefaults(&rgb, image);
    rgb.format = AVIF_RGB_FORMAT_RGBA;
    rgb.depth = 8;
    rgb.pixels = (uint8_t *)rgba;
    rgb.rowBytes = stride;
    rgb.ignoreAlpha = opaque ? AVIF_TRUE : AVIF_FALSE;
    rgb.alphaPremultiplied = (!opaque && premultiplied) ? AVIF_TRUE : AVIF_FALSE;
    avifResult result = avifImageRGBToYUV(image, &rgb);

    avifEncoder *encoder = result == AVIF_RESULT_OK ? avifEncoderCreate() : NULL;
    if (result == AVIF_RESULT_OK && !encoder) result = AVIF_RESULT_OUT_OF_MEMORY;
    if (encoder) {
        encoder->codecChoice = AVIF_CODEC_CHOICE_AOM;
        encoder->quality = quality;
        // Alpha, when present, stays lossless (libavif's and avifenc's default).
        encoder->qualityAlpha = AVIF_QUALITY_LOSSLESS;
        encoder->speed = speed;
        encoder->maxThreads = threads < 1 ? 1 : threads;
        result = avifEncoderSetCodecSpecificOption(encoder, "color:tune", CAPER_AVIF_TUNE);
        if (result == AVIF_RESULT_OK) result = avifEncoderWrite(encoder, image, output);
        avifEncoderDestroy(encoder);
    }
    avifImageDestroy(image);
    if (result != AVIF_RESULT_OK) avifRWDataFree(output);
    return result;
}

#ifdef __ANDROID__
#include <android/bitmap.h>
#include <jni.h>

JNIEXPORT jbyteArray JNICALL Java_chat_caper_android_data_AvifEncoder_nativeEncode(
    JNIEnv *env, jclass clazz, jobject bitmap, jboolean premultiplied, jint colorPrimaries, jint quality, jint speed, jint threads) {
    (void)clazz;
    AndroidBitmapInfo info;
    if (AndroidBitmap_getInfo(env, bitmap, &info) != ANDROID_BITMAP_RESULT_SUCCESS || info.format != ANDROID_BITMAP_FORMAT_RGBA_8888) return NULL;
    void *pixels = NULL;
    if (AndroidBitmap_lockPixels(env, bitmap, &pixels) != ANDROID_BITMAP_RESULT_SUCCESS || !pixels) return NULL;
    avifRWData output = AVIF_DATA_EMPTY;
    avifResult result = caper_avif_encode((const uint8_t *)pixels, info.width, info.height, info.stride, premultiplied == JNI_TRUE,
                                          colorPrimaries, quality, speed, threads, &output);
    AndroidBitmap_unlockPixels(env, bitmap);
    jbyteArray encoded = NULL;
    if (result == AVIF_RESULT_OK && output.size > 0 && output.size <= 0x7fffffff) {
        encoded = (*env)->NewByteArray(env, (jsize)output.size);
        if (encoded) (*env)->SetByteArrayRegion(env, encoded, 0, (jsize)output.size, (const jbyte *)output.data);
    }
    avifRWDataFree(&output);
    return encoded;
}
#endif
