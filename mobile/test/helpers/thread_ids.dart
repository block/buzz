// Valid event IDs for thread fixtures. Names retain the scenario vocabulary.
const threadIdA =
    'cda64cca437003cfa45e048e657ea16d84c976258dbf2bcc9b8671da7ed780fc';
const threadIdBotDashMessage =
    'c8e8258450906baf898a88c29afec447315d586e1693f05dfe0a71eaafd7c2e1';
const threadIdBroadcast =
    '6bc5ac314996ff5d95c1a60078115d395ecb501cf6d715a9e91cb9187f9606dd';
const threadIdLocalDashRoot =
    'd80a1acb7655fc14d04097c8f8de30ac8b0735101d3d6f861c4a0c70033803e4';
const threadIdMid =
    '231d7ca8ae13bac3ebd696c5c716423c07ab85d9d49ca8d4865ec7a8f7a35235';
const threadIdNewDashRoot =
    'e6d295e744f0050d61680f203b637c76f8b3c2b631b78ad4ed13e6df81847e42';
const threadIdNewUnderscoreParent =
    '7717c240593ebc319e5c47af9dc26137b3d9183fbe259b7dc70076ecd3d3dc4a';
const threadIdOldDashRoot =
    '13508ffa8f8c267ef347c06ba6c523d29bac5febba8d9c86101c3e7473cc903e';
const threadIdOldUnderscoreParent =
    '870f30951b8a58571f95c17b866b91b4cfd8df9c3f9800f2162842c234710a3a';
const threadIdOuterDashRoot =
    '5f2441c40dfa4031b611a769d5480af8190b36afe934dbbfed78c0e06f4f7567';
const threadIdOwner =
    '07d3c362dbb52cf541a0c6fa0f66c194d6a3708038439cc35d41be42cda683ab';
const threadIdParent =
    '35e2659925fc3ed5d1433f37772bb6d3ff5062aa557bb2a3c4aa8206cceb3e59';
const threadIdParent1 =
    '052ae53ff85a8c9b02a6699b0792e2d88fea6773e71260e4c30d58b4db3a558d';
const threadIdParentDashNote =
    '6b512f2db72703829a14f28d84f313dc836f9ebb028102476f7628c6cbfd99b0';
const threadIdParentDashReply =
    '9fd28ae5a15a58b792157e5ef689c4bc47e3c0fbb943976ca6066d8d60a6443f';
const threadIdR1 =
    'ebf874a4b0b1b2eae28b3eac554f895bdd0a5821d5d546c4ce8869075287c2c7';
const threadIdRefreshDashRoot =
    'ee0e6ebc1b13e2751752060106c2d1c99e52dcd5feb7a917de0788f4a831c8b6';
const threadIdRetained =
    '89de9f0810a1ee27ca48249b067a2ad1aeea433a04ff8fcd9ce87027c606726f';
const threadIdRoot =
    '70fcdcb3018461973b5ab30b8485650f4b5a5a917d14ba8471de62f4e312b32d';
const threadIdRoot1 =
    '5feade3971cffc7089962f5386f38ebce3e214a3fc11e7d5e03e76ebb3fb3b76';
const threadIdRootDash1 =
    '722ee7881f4a2e36157247f7b555c7fd48154928613d628d3141ecb2da175eb9';
const threadIdRootDash2 =
    '51196df818f9c90509ae9aadb4e1d67a8a34bc12cdec683afa7526cfaa6f2d3d';
const threadIdRootDash9 =
    'decac4ac88ce84fff8e6cb46def78cbb2da80a49ce38a2cb2c9e1a334b4b059f';
const threadIdStickyDashThreadDashRoot =
    '535e416282f60dfd00cb81d34effdd6902f128c8c75c8186777e4fe11349b39b';
const threadIdTallDashRoot =
    'e42e600a65b80dcb5e4d6a444bfb5359266d88979a628bb6b0d76ceaaa256b0b';
const threadIdTapDashRoot =
    'f29babc532b7cceaabacb531fa5b61715c13933e6a511d29b897dde23af7ccc9';
const threadIdThreadDashRoot =
    '6a431d1af075d290deddf39e5018c90250f7301c5ce9abaed28ed4ec95271080';
const threadIdTopDashRoot =
    '0c44b7e3be2808282a9f11ec9a144aff460cab80c3b14fb99867c834411d6fa3';
const threadIdUnrelated =
    'd0ce3e35b5a76a22e9c81dbd60843672d36906033406a3640279f97c16fa361a';
const threadIdVideoDashRoot =
    'eca034e64102b0ef0a4c5202afbef5570e4229ae28b342fc9383107a6e173f07';

const threadIdReplyZero =
    '7265706c792d3000000000000000000000000000000000000000000000000000';

String indexedThreadId(String label) {
  assert(label.length <= 32);
  return label.codeUnits
      .map((c) => c.toRadixString(16).padLeft(2, '0'))
      .join()
      .padRight(64, '0');
}

const threadIdOtherRoot =
    'e54aad59fd94333b15eb79e7fd803fb12d0572fd15eda9c14c2d53240cb77ea6';

const threadIdInboxRoot1 =
    '5feade3971cffc7089962f5386f38ebce3e214a3fc11e7d5e03e76ebb3fb3b76';

const threadIdInboxTop =
    '84c18f776617e2acec24db8ad4b4f76271bfcec8777cc608b3594ddaf1e319f5';

const threadIdInboxA =
    'cda64cca437003cfa45e048e657ea16d84c976258dbf2bcc9b8671da7ed780fc';

const threadIdInboxP =
    'be916ad31aba19f29c5d2746a1e5fae3baab44ca81b9c5ce795e43f71e45d490';

const threadIdInboxB =
    '255f9f032274c31cf0d52b3005e8f29b42d89d5bb4ffa30645b9415e4d584490';

const threadIdInboxR =
    '30961e01b3b182917abeac529df1ad972f51f4b344fa4f918a280696c807155b';
