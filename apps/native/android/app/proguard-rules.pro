-keep class org.webrtc.** { *; }
-dontwarn org.webrtc.**
# WebRTC's JNI_OnLoad looks up org.jni_zero.JniZero by name, and the SDK ships no
# consumer rules for it. Without this, R8 renames the class and the release build
# aborts ("java_class == null") the first time voice loads WebRTC.
-keep class org.jni_zero.** { *; }
-dontwarn org.jni_zero.**
