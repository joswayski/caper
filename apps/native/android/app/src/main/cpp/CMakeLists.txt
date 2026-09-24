cmake_minimum_required(VERSION 3.22.1)
project(caper_audio LANGUAGES C CXX)
set(CMAKE_CXX_STANDARD 17)
# CI/orbs can have many reported CPUs but little RAM. ONNX headers and the
# RNNoise model source must not spawn one compiler per reported core.
set_property(GLOBAL PROPERTY JOB_POOLS caper_compile=1)
set(CMAKE_JOB_POOL_COMPILE caper_compile)
set(DEPENDENCIES "${CMAKE_CURRENT_LIST_DIR}/../../../build/native-inputs")
set(RN "${DEPENDENCIES}/rnnoise")
set(ORT "${DEPENDENCIES}/ort")

add_library(onnxruntime SHARED IMPORTED)
set_target_properties(onnxruntime PROPERTIES IMPORTED_LOCATION "${ORT}/jni/${ANDROID_ABI}/libonnxruntime.so")
add_library(caper_audio SHARED AudioCapture.cpp CaperDpdfnet.cpp CaperDenoisePipeline.cpp CaperVoiceDSP.c
    "${RN}/src/denoise.c" "${RN}/src/rnn.c" "${RN}/src/pitch.c" "${RN}/src/kiss_fft.c"
    "${RN}/src/celt_lpc.c" "${RN}/src/nnet.c" "${RN}/src/nnet_default.c"
    "${RN}/src/parse_lpcnet_weights.c" "${RN}/src/rnnoise_data.c" "${RN}/src/rnnoise_tables.c")
target_include_directories(caper_audio PRIVATE "${ORT}/headers" "${RN}/include" "${RN}/src")
target_compile_definitions(caper_audio PRIVATE RNNOISE_BUILD=1 DISABLE_DEBUG_FLOAT=1)
target_link_options(caper_audio PRIVATE "-Wl,-z,max-page-size=16384" "-Wl,-z,common-page-size=16384")
target_link_libraries(caper_audio PRIVATE onnxruntime log m)
