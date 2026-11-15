#!/bin/bash
# usage: proto-setup.sh SERIAL
export ANDROID_HOME=/opt/homebrew/share/android-commandlinetools; export PATH=$ANDROID_HOME/platform-tools:$PATH
S=$1; P=xyz.block.buzz.mobile.androidpushproto
APK=~/.buzz/REPOS/buzz-android-push-proto/mobile/build/app/outputs/flutter-apk/app-release.apk
adb -s $S install -r $APK | tail -1
adb -s $S shell pm grant $P android.permission.POST_NOTIFICATIONS
adb -s $S reverse tcp:3100 tcp:3100
adb -s $S shell am broadcast -n $P/xyz.block.buzz.mobile.PushProtoReceiver -a xyz.block.buzz.mobile.PROTO_SEED --es relay http://localhost:3100 --es nsec $(cat /tmp/proto-nsec.txt) >/dev/null
sleep 5
adb -s $S logcat -d -s PUSHPROTO | grep -E "finish" 
