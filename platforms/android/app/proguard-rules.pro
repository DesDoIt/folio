# JNA and the UniFFI-generated bindings are reached by reflection from native code.
-keep class com.sun.jna.** { *; }
-keepclassmembers class * extends com.sun.jna.** { public *; }
-keep class uniffi.engine_ffi.** { *; }
-dontwarn java.awt.**
