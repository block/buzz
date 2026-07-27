# Android Bitmap media fixtures

These 3 x 2 fixtures were produced by Android 16 (API 36) `Bitmap.compress`, not by a generic image encoder.

## Regeneration

1. Compile and run the following program on an API 36 emulator:

   ```java
   import android.graphics.Bitmap;
   import android.graphics.Color;
   import android.graphics.ColorSpace;
   import java.io.FileOutputStream;

   public final class Main {
     private static void write(Bitmap bitmap, Bitmap.CompressFormat format, String stem)
         throws Exception {
       String extension = format == Bitmap.CompressFormat.PNG ? "png" : "jpg";
       try (FileOutputStream output =
           new FileOutputStream("/data/local/tmp/" + stem + "." + extension)) {
         if (!bitmap.compress(format, 100, output)) {
           throw new IllegalStateException("Bitmap.compress failed for " + stem);
         }
       }
     }

     public static void main(String[] args) throws Exception {
       Bitmap srgb = Bitmap.createBitmap(3, 2, Bitmap.Config.ARGB_8888);
       srgb.setPixels(new int[] {
         Color.argb(255, 255, 0, 0), Color.argb(255, 0, 255, 0),
         Color.argb(255, 0, 0, 255), Color.argb(128, 255, 255, 0),
         Color.argb(64, 0, 255, 255), Color.argb(0, 255, 0, 255),
       }, 0, 3, 0, 0, 3, 2);
       write(srgb, Bitmap.CompressFormat.PNG, "bitmap-srgb");
       write(srgb, Bitmap.CompressFormat.JPEG, "bitmap-srgb");

       Bitmap displayP3 = Bitmap.createBitmap(
           3, 2, Bitmap.Config.RGBA_F16, true,
           ColorSpace.get(ColorSpace.Named.DISPLAY_P3));
       displayP3.eraseColor(Color.pack(
           1.0f, 0.0f, 0.0f, 1.0f,
           ColorSpace.get(ColorSpace.Named.DISPLAY_P3)));
       write(displayP3, Bitmap.CompressFormat.PNG, "bitmap-display-p3");
       write(displayP3, Bitmap.CompressFormat.JPEG, "bitmap-display-p3");
     }
   }
   ```

2. Pull the four files from `/data/local/tmp/` into this directory. Copy all four into `mobile/android/app/src/test/resources/fixtures/android/`, and copy `bitmap-display-p3.png` and `bitmap-srgb.png` into `mobile/android/app/src/androidTest/resources/fixtures/android/`.
3. Run `cmp` on every copied fixture to confirm it is byte-identical.
4. Run the app's `AndroidImageProcessor.decodeSrgbBitmap` and `encodeAndScrub` path for both source color spaces and both formats on the API 36 emulator. Save the four outputs under `sanitized/` with the `-sanitized` suffix.
5. Run `cargo test -p buzz-media android_` to verify the relay accepts every sanitized fixture while rejecting the three unsanitized fixtures that contain forbidden metadata.
6. Run `cd mobile/android && ./gradlew app:testDebugUnitTest app:connectedDebugAndroidTest` with an API 36 emulator to verify structural scrubbing and the Display-P3 to sRGB conversion for sanitized PNG and JPEG output.

Regenerate both the encoded inputs and sanitized outputs whenever Android `Bitmap.compress` behavior or `AndroidMediaSanitizer` changes. Do not update only the sanitized files, because the tests cover the exact encoder-to-sanitizer boundary.
