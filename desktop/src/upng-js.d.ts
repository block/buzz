declare module "upng-js" {
  const UPNG: {
    /**
     * Encode RGBA frames as a (possibly animated) PNG.
     *
     * @param imgs - one RGBA8 ArrayBuffer per frame
     * @param cnum - color count for lossy quantization; 0 = lossless
     * @param dels - per-frame delays in milliseconds (animated when > 1 frame)
     */
    encode(
      imgs: ArrayBuffer[],
      width: number,
      height: number,
      cnum: number,
      dels?: number[],
    ): ArrayBuffer;
  };
  export default UPNG;
}
