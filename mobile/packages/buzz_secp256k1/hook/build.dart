import 'package:code_assets/code_assets.dart';
import 'package:hooks/hooks.dart';
import 'package:native_toolchain_c/native_toolchain_c.dart';

const _vendor = 'third_party/secp256k1';

void main(List<String> args) async {
  await build(args, (input, output) async {
    if (!input.config.buildCodeAssets) return;
    await CBuilder.library(
      name: 'buzz_secp256k1',
      assetName: 'src/bindings.dart',
      sources: [
        '$_vendor/src/secp256k1.c',
        '$_vendor/src/precomputed_ecmult.c',
        '$_vendor/src/precomputed_ecmult_gen.c',
        'src/buzz_secp256k1_ecdh.c',
      ],
      // Mirrors upstream CMake Release defaults for v0.8.0 (-O2 rather than
      // CBuilder's -O3 default). The precomputed tables
      // are generated for exactly these window and comb parameters.
      optimizationLevel: OptimizationLevel.o2,
      defines: {
        // Export the API from the Windows DLL; no effect elsewhere.
        'SECP256K1_DLL_EXPORT': '1',
        'ENABLE_MODULE_ECDH': '1',
        'ENABLE_MODULE_EXTRAKEYS': '1',
        'ENABLE_MODULE_SCHNORRSIG': '1',
        'ECMULT_WINDOW_SIZE': '15',
        'COMB_BLOCKS': '43',
        'COMB_TEETH': '6',
      },
    ).run(input: input, output: output);
  });
}
