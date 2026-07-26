#import <AVFoundation/AVFoundation.h>
#import <AudioToolbox/AudioToolbox.h>
#import <Foundation/Foundation.h>
#import <dlfcn.h>
#import <objc/runtime.h>

typedef void (*BuzzSiriPCMCallback)(
    void *context,
    const float *samples,
    uint32_t frameCount,
    double sampleRate
);
typedef void (*BuzzSiriCompletionCallback)(void *context, const char *error);

@protocol BuzzSiriDaemonProtocol <NSObject>
- (void)cancelWithRequest:(id)request;
- (void)synthesizeWithRequest:(id)request
                        reply:(void (^)(NSError *error))reply;
@end

@protocol BuzzSiriAvailabilityProtocol <NSObject>
- (void)downloadedVoicesMatching:(id)voice
                           reply:(void (^)(NSArray *voices))reply;
@end

@protocol BuzzSiriSubscribeProtocol <NSObject>
- (void)subscribeWithVoices:(NSArray *)voices
                   clientId:(NSString *)clientId
                accessoryId:(NSString *)accessoryId
                      reply:(void (^)(NSError *error))reply;
@end

@protocol BuzzSiriSessionDelegate <NSObject>
- (void)didGenerateAudioWithRequestId:(uint64_t)requestId audio:(id)audio;
- (void)didGenerateWordTimingsWithRequestId:(uint64_t)requestId
                             wordTimingInfo:(id)info;
- (void)didReportInstrumentWithRequestId:(uint64_t)requestId
                   instrumentationMetrics:(id)metrics;
- (void)didStartSpeakingWithRequestId:(uint64_t)requestId;
- (void)pingWithReply:(void (^)(void))reply;
- (void)event:(uint64_t)event eventData:(id)data;
- (void)internalEvent:(uint64_t)event internalEventData:(id)data;
@end

static BOOL BuzzLoadFramework(NSString *path) {
    return dlopen(path.UTF8String, RTLD_NOW) != NULL;
}

static BOOL BuzzLoadSiriTTS(void) {
    return BuzzLoadFramework(
        @"/System/Library/PrivateFrameworks/SiriTTSService.framework/SiriTTSService"
    );
}

static id BuzzCreateVoice(NSString *language, NSString *name) {
    if (!BuzzLoadSiriTTS()) return nil;
    Class cls = objc_getClass("SiriTTSSynthesisVoice");
    if (!cls) return nil;
    return [[cls alloc] performSelector:@selector(initWithLanguage:name:)
                             withObject:language
                             withObject:name];
}

static NSSet<Class> *BuzzRequestClasses(void) {
    NSMutableSet<Class> *classes = [NSMutableSet setWithObjects:
        NSString.class, NSNumber.class, NSData.class, NSURL.class,
        NSUUID.class, NSValue.class, NSArray.class, NSDictionary.class,
        NSError.class, nil];
    for (NSString *name in @[
        @"SiriTTSSynthesisRequest", @"SiriTTSSpeechRequest",
        @"SiriTTSSynthesisVoice", @"SiriTTSBaseRequest",
        @"SiriTTSAudibleContext", @"SiriTTSSynthesisContext",
        @"SiriTTSProsodyProperties", @"SiriTTSAudioData",
        @"SiriTTSWordTimingInfo", @"SiriTTSInstrumentationMetrics"
    ]) {
        Class cls = objc_getClass(name.UTF8String);
        if (cls) [classes addObject:cls];
    }
    return classes;
}

static char *BuzzCopyJSONString(id value) {
    if (!value) return NULL;
    NSData *data = [NSJSONSerialization dataWithJSONObject:value options:0 error:nil];
    if (!data) return NULL;
    char *copy = malloc(data.length + 1);
    if (!copy) return NULL;
    memcpy(copy, data.bytes, data.length);
    copy[data.length] = '\0';
    return copy;
}

static NSDictionary *BuzzVoiceDictionary(id voice) {
    return @{
        @"language": [voice valueForKey:@"language"] ?: @"",
        @"name": [voice valueForKey:@"name"] ?: @"",
        @"version": [voice valueForKey:@"version"] ?: @0,
    };
}

@interface BuzzSiriDelegateImpl : NSObject <BuzzSiriSessionDelegate>
@property(nonatomic, assign) void *context;
@property(nonatomic, assign) BuzzSiriPCMCallback callback;
@property(nonatomic, strong) AVAudioConverter *converter;
@property(nonatomic, strong) AVAudioFormat *sourceFormat;
- (void)deactivate;
@end

@implementation BuzzSiriDelegateImpl

- (AVAudioPCMBuffer *)decodeAudio:(id)audio error:(NSError **)error {
    NSData *data = [audio valueForKey:@"audioData"];
    if (!data.length) return nil;
    NSNumber *packetCountValue = [audio valueForKey:@"packetCount"] ?: @1;
    NSData *packetDescriptions = [audio valueForKey:@"packetDescriptions"];
    SEL selector = NSSelectorFromString(@"asbd");
    typedef AudioStreamBasicDescription (*GetASBD)(id, SEL);
    GetASBD getASBD = (GetASBD)[audio methodForSelector:selector];
    AudioStreamBasicDescription description = getASBD(audio, selector);
    AVAudioFormat *source = [[AVAudioFormat alloc] initWithStreamDescription:&description];
    if (!source) return nil;

    AVAudioFormat *destination = [[AVAudioFormat alloc]
        initWithCommonFormat:AVAudioPCMFormatFloat32
                  sampleRate:description.mSampleRate
                    channels:description.mChannelsPerFrame
                 interleaved:NO];
    if (!destination) return nil;

    AVAudioBuffer *input = nil;
    if (description.mFormatID == kAudioFormatLinearPCM) {
        UInt32 bytesPerFrame = description.mBytesPerFrame;
        if (!bytesPerFrame) return nil;
        AVAudioFrameCount frames = (AVAudioFrameCount)(data.length / bytesPerFrame);
        AVAudioPCMBuffer *pcm = [[AVAudioPCMBuffer alloc]
            initWithPCMFormat:source frameCapacity:frames];
        pcm.frameLength = frames;
        AudioBufferList *list = pcm.mutableAudioBufferList;
        if (!list || !list->mNumberBuffers) return nil;
        NSUInteger remaining = data.length;
        const uint8_t *sourceBytes = data.bytes;
        for (UInt32 index = 0; index < list->mNumberBuffers && remaining; index++) {
            AudioBuffer *buffer = &list->mBuffers[index];
            NSUInteger count = MIN(remaining, buffer->mDataByteSize);
            memcpy(buffer->mData, sourceBytes, count);
            sourceBytes += count;
            remaining -= count;
        }
        input = pcm;
    } else if (description.mFormatID == kAudioFormatOpus) {
        AVAudioPacketCount packetCount = MAX(packetCountValue.unsignedIntValue, 1);
        AVAudioCompressedBuffer *compressed = [[AVAudioCompressedBuffer alloc]
            initWithFormat:source
            packetCapacity:packetCount
            maximumPacketSize:MAX(data.length, 1)];
        compressed.byteLength = (UInt32)data.length;
        compressed.packetCount = packetCount;
        memcpy(compressed.data, data.bytes, data.length);
        if (packetDescriptions.length && compressed.packetDescriptions) {
            NSUInteger required =
                packetCount * sizeof(AudioStreamPacketDescription);
            if (packetDescriptions.length < required) return nil;
            memcpy(compressed.packetDescriptions, packetDescriptions.bytes, required);
        } else if (packetCount == 1 && compressed.packetDescriptions) {
            compressed.packetDescriptions[0] = (AudioStreamPacketDescription){
                .mStartOffset = 0,
                .mVariableFramesInPacket = 0,
                .mDataByteSize = (UInt32)data.length,
            };
        } else {
            return nil;
        }
        input = compressed;
    } else {
        return nil;
    }

    if (!self.converter ||
        ![self.sourceFormat isEqual:source]) {
        self.converter = [[AVAudioConverter alloc] initFromFormat:source
                                                        toFormat:destination];
        self.sourceFormat = source;
    }
    if (!self.converter) return nil;

    AVAudioFrameCount capacity =
        MAX((AVAudioFrameCount)packetCountValue.unsignedIntValue * 5760, 4096);
    if ([input isKindOfClass:AVAudioPCMBuffer.class]) {
        capacity = ((AVAudioPCMBuffer *)input).frameLength + 32;
    }
    AVAudioPCMBuffer *output = [[AVAudioPCMBuffer alloc]
        initWithPCMFormat:destination frameCapacity:capacity];
    __block BOOL supplied = NO;
    AVAudioConverterOutputStatus status = [self.converter
        convertToBuffer:output
                  error:error
     withInputFromBlock:^AVAudioBuffer *(AVAudioPacketCount requested,
                                         AVAudioConverterInputStatus *inputStatus) {
        if (supplied) {
            *inputStatus = AVAudioConverterInputStatus_NoDataNow;
            return nil;
        }
        supplied = YES;
        *inputStatus = AVAudioConverterInputStatus_HaveData;
        return input;
    }];
    if (status == AVAudioConverterOutputStatus_Error || !output.frameLength) {
        return nil;
    }
    return output;
}

- (void)didGenerateAudioWithRequestId:(uint64_t)requestId audio:(id)audio {
    @synchronized (self) {
        NSError *error = nil;
        AVAudioPCMBuffer *buffer = [self decodeAudio:audio error:&error];
        if (!buffer || !self.callback || !buffer.floatChannelData) return;
        self.callback(
            self.context,
            buffer.floatChannelData[0],
            buffer.frameLength,
            buffer.format.sampleRate
        );
    }
}

- (void)deactivate {
    @synchronized (self) {
        self.callback = NULL;
        self.context = NULL;
    }
}

- (void)didGenerateWordTimingsWithRequestId:(uint64_t)requestId
                             wordTimingInfo:(id)info {}
- (void)didReportInstrumentWithRequestId:(uint64_t)requestId
                   instrumentationMetrics:(id)metrics {}
- (void)didStartSpeakingWithRequestId:(uint64_t)requestId {}
- (void)pingWithReply:(void (^)(void))reply { if (reply) reply(); }
- (void)event:(uint64_t)event eventData:(id)data {}
- (void)internalEvent:(uint64_t)event internalEventData:(id)data {}
@end

@interface BuzzSiriSession : NSObject
@property(nonatomic, strong) NSXPCConnection *connection;
@property(nonatomic, strong) BuzzSiriDelegateImpl *delegate;
@property(nonatomic, strong) id request;
@property(nonatomic, assign) void *context;
@property(nonatomic, assign) BuzzSiriCompletionCallback completion;
@property(nonatomic, assign) BOOL finished;
@end

@implementation BuzzSiriSession
- (void)finish:(NSError *)error {
    @synchronized (self) {
        if (self.finished) return;
        self.finished = YES;
    }
    [self.delegate deactivate];
    if (self.completion) {
        self.completion(
            self.context,
            error ? error.localizedDescription.UTF8String : NULL
        );
    }
}

- (void)beginCancellation {
    @synchronized (self) {
        if (self.finished) return;
        self.finished = YES;
    }
    [self.delegate deactivate];
}

- (void)dealloc {
    [self.delegate deactivate];
    [self.connection invalidate];
}
@end

void *buzz_siri_session_create(
    void *context,
    BuzzSiriPCMCallback audioCallback,
    BuzzSiriCompletionCallback completionCallback
) {
    BuzzSiriSession *session = [BuzzSiriSession new];
    session.context = context;
    session.completion = completionCallback;
    session.delegate = [BuzzSiriDelegateImpl new];
    session.delegate.context = context;
    session.delegate.callback = audioCallback;
    return (__bridge_retained void *)session;
}

void buzz_siri_session_synthesize(
    void *opaque,
    const char *textValue,
    const char *languageValue,
    const char *voiceValue,
    float rate
) {
    BuzzSiriSession *session = (__bridge BuzzSiriSession *)opaque;
    NSString *text = [NSString stringWithUTF8String:textValue];
    NSString *language = [NSString stringWithUTF8String:languageValue];
    NSString *voiceName = [NSString stringWithUTF8String:voiceValue];
    id voice = BuzzCreateVoice(language, voiceName);
    Class requestClass = objc_getClass("SiriTTSSynthesisRequest");
    if (!voice || !requestClass) {
        [session finish:[NSError errorWithDomain:@"com.block.buzz.sirittsd"
                                            code:1
                                        userInfo:@{NSLocalizedDescriptionKey:
                                            @"Siri TTS is unavailable on this Mac."}]];
        return;
    }
    id request = [[requestClass alloc]
        performSelector:@selector(initWithText:voice:)
        withObject:text
        withObject:voice];
    if (rate != 1.0f && [request respondsToSelector:@selector(setRate:)]) {
        typedef void (*SetFloat)(id, SEL, float);
        SetFloat setRate = (SetFloat)[request methodForSelector:@selector(setRate:)];
        setRate(request, @selector(setRate:), rate);
    }

    NSXPCConnection *connection = [[NSXPCConnection alloc]
        initWithMachServiceName:@"com.apple.sirittsd" options:0];
    NSXPCInterface *remote = [NSXPCInterface
        interfaceWithProtocol:@protocol(BuzzSiriDaemonProtocol)];
    NSSet<Class> *classes = BuzzRequestClasses();
    [remote setClasses:classes
            forSelector:@selector(synthesizeWithRequest:reply:)
          argumentIndex:0
                ofReply:NO];
    [remote setClasses:classes
            forSelector:@selector(cancelWithRequest:)
          argumentIndex:0
                ofReply:NO];
    connection.remoteObjectInterface = remote;
    NSXPCInterface *exported = [NSXPCInterface
        interfaceWithProtocol:@protocol(BuzzSiriSessionDelegate)];
    [exported setClasses:classes
             forSelector:@selector(didGenerateAudioWithRequestId:audio:)
           argumentIndex:1
                 ofReply:NO];
    [exported setClasses:classes
             forSelector:@selector(didGenerateWordTimingsWithRequestId:wordTimingInfo:)
           argumentIndex:1
                 ofReply:NO];
    [exported setClasses:classes
             forSelector:@selector(didReportInstrumentWithRequestId:instrumentationMetrics:)
           argumentIndex:1
                 ofReply:NO];
    connection.exportedInterface = exported;
    connection.exportedObject = session.delegate;
    session.connection = connection;
    session.request = request;
    __weak BuzzSiriSession *weakSession = session;
    connection.interruptionHandler = ^{
        [weakSession finish:[NSError errorWithDomain:@"com.block.buzz.sirittsd"
                                                code:2
                                            userInfo:@{NSLocalizedDescriptionKey:
                                                @"Siri TTS connection interrupted."}]];
    };
    connection.invalidationHandler = ^{
        [weakSession finish:[NSError errorWithDomain:@"com.block.buzz.sirittsd"
                                                code:3
                                            userInfo:@{NSLocalizedDescriptionKey:
                                                @"Siri TTS connection invalidated."}]];
    };
    [connection resume];
    id<BuzzSiriDaemonProtocol> proxy =
        [connection remoteObjectProxyWithErrorHandler:^(NSError *error) {
            [weakSession finish:error];
        }];
    [proxy synthesizeWithRequest:request reply:^(NSError *error) {
        [weakSession finish:error];
    }];
}

void buzz_siri_session_cancel(void *opaque) {
    BuzzSiriSession *session = (__bridge BuzzSiriSession *)opaque;
    [session beginCancellation];
    if (session.connection && session.request) {
        id<BuzzSiriDaemonProtocol> proxy =
            [session.connection remoteObjectProxyWithErrorHandler:^(NSError *error) {}];
        [proxy cancelWithRequest:session.request];
    }
    [session.connection invalidate];
}

void buzz_siri_session_release(void *opaque) {
    if (opaque) CFRelease(opaque);
}

char *buzz_siri_discover_voices_json(const char *prefixValue) {
    @autoreleasepool {
        if (!BuzzLoadFramework(
            @"/System/Library/PrivateFrameworks/TextToSpeech.framework/TextToSpeech"
        )) return BuzzCopyJSONString(@[]);
        Class managerClass = objc_getClass("TTSAXResourceManager");
        if (!managerClass) return BuzzCopyJSONString(@[]);
        id manager = [managerClass performSelector:@selector(sharedInstance)];
        NSArray *resources = [manager performSelector:@selector(allVoices:)
                                            withObject:nil];
        NSString *prefix = [[NSString stringWithUTF8String:prefixValue ?: ""]
            stringByReplacingOccurrencesOfString:@"_" withString:@"-"];
        NSMutableDictionary *byKey = [NSMutableDictionary dictionary];
        for (id resource in resources) {
            NSString *identifier = [resource valueForKey:@"identifier"];
            if (![identifier hasPrefix:@"com.apple.siri.natural."]) continue;
            NSString *language = [resource valueForKey:@"language"] ?: @"";
            if (prefix.length &&
                ![language.lowercaseString hasPrefix:prefix.lowercaseString]) continue;
            NSString *name = [resource valueForKey:@"name"] ?: @"";
            if (!name.length || !language.length) continue;
            NSString *key = [NSString stringWithFormat:@"%@|%@",
                name.lowercaseString, language.lowercaseString];
            byKey[key] = @{
                @"name": name,
                @"language": language,
                @"identifier": identifier,
                @"size_bytes": [resource valueForKey:@"assetSize"] ?: @0,
            };
        }
        NSArray *voices = [[byKey allValues] sortedArrayUsingComparator:
            ^NSComparisonResult(NSDictionary *left, NSDictionary *right) {
                NSComparisonResult language =
                    [left[@"language"] localizedCaseInsensitiveCompare:right[@"language"]];
                if (language != NSOrderedSame) return language;
                return [left[@"name"] localizedCaseInsensitiveCompare:right[@"name"]];
            }];
        return BuzzCopyJSONString(voices);
    }
}

char *buzz_siri_downloaded_voices_json(
    const char *languageValue,
    const char *voiceValue
) {
    @autoreleasepool {
        NSString *language = [NSString stringWithUTF8String:languageValue];
        NSString *voiceName = [NSString stringWithUTF8String:voiceValue];
        id voice = BuzzCreateVoice(language, voiceName);
        if (!voice) return NULL;
        NSXPCConnection *connection = [[NSXPCConnection alloc]
            initWithMachServiceName:@"com.apple.sirittsd" options:0];
        NSXPCInterface *interface = [NSXPCInterface
            interfaceWithProtocol:@protocol(BuzzSiriAvailabilityProtocol)];
        NSSet<Class> *classes = BuzzRequestClasses();
        [interface setClasses:classes
                  forSelector:@selector(downloadedVoicesMatching:reply:)
                argumentIndex:0
                      ofReply:NO];
        [interface setClasses:classes
                  forSelector:@selector(downloadedVoicesMatching:reply:)
                argumentIndex:0
                      ofReply:YES];
        connection.remoteObjectInterface = interface;
        [connection resume];
        dispatch_semaphore_t semaphore = dispatch_semaphore_create(0);
        __block NSArray *result = nil;
        id<BuzzSiriAvailabilityProtocol> proxy =
            [connection remoteObjectProxyWithErrorHandler:^(NSError *error) {
                dispatch_semaphore_signal(semaphore);
            }];
        [proxy downloadedVoicesMatching:voice reply:^(NSArray *voices) {
            NSMutableArray *mapped = [NSMutableArray array];
            for (id resolved in voices) [mapped addObject:BuzzVoiceDictionary(resolved)];
            result = mapped;
            dispatch_semaphore_signal(semaphore);
        }];
        dispatch_semaphore_wait(
            semaphore,
            dispatch_time(DISPATCH_TIME_NOW, 3 * NSEC_PER_SEC)
        );
        [connection invalidate];
        return BuzzCopyJSONString(result ?: @[]);
    }
}

int buzz_siri_trigger_voice_download(
    const char *languageValue,
    const char *voiceValue
) {
    @autoreleasepool {
        NSString *language = [NSString stringWithUTF8String:languageValue];
        NSString *voiceName = [NSString stringWithUTF8String:voiceValue];
        id voice = BuzzCreateVoice(language, voiceName);
        if (!voice) return 0;
        NSXPCConnection *connection = [[NSXPCConnection alloc]
            initWithMachServiceName:@"com.apple.sirittsd" options:0];
        connection.remoteObjectInterface = [NSXPCInterface
            interfaceWithProtocol:@protocol(BuzzSiriSubscribeProtocol)];
        [connection resume];
        dispatch_semaphore_t semaphore = dispatch_semaphore_create(0);
        __block BOOL subscribed = NO;
        id<BuzzSiriSubscribeProtocol> proxy =
            [connection remoteObjectProxyWithErrorHandler:^(NSError *error) {
                dispatch_semaphore_signal(semaphore);
            }];
        [proxy subscribeWithVoices:@[voice]
                         clientId:@"com.apple.speech"
                      accessoryId:@""
                            reply:^(NSError *error) {
            subscribed = error == nil;
            dispatch_semaphore_signal(semaphore);
        }];
        dispatch_semaphore_wait(
            semaphore,
            dispatch_time(DISPATCH_TIME_NOW, 10 * NSEC_PER_SEC)
        );
        [connection invalidate];
        if (!subscribed) return 0;

        if (!BuzzLoadFramework(
            @"/System/Library/PrivateFrameworks/UnifiedAssetFramework.framework/UnifiedAssetFramework"
        )) return 0;
        Class serviceClass = objc_getClass("UAFAssetUtilitiesService");
        id service = serviceClass ? [serviceClass new] : nil;
        SEL switchSelector = NSSelectorFromString(@"switchLanguage:");
        SEL downloadSelector = NSSelectorFromString(@"downloadSiriAssets");
        if (![service respondsToSelector:switchSelector] ||
            ![service respondsToSelector:downloadSelector]) return 0;
        NSString *normalized =
            [language stringByReplacingOccurrencesOfString:@"-" withString:@"_"];
        typedef void (*SendObject)(id, SEL, id);
        typedef void (*SendVoid)(id, SEL);
        ((SendObject)[service methodForSelector:switchSelector])(
            service, switchSelector, normalized
        );
        ((SendVoid)[service methodForSelector:downloadSelector])(
            service, downloadSelector
        );
        return 1;
    }
}

void buzz_siri_free_string(char *value) {
    free(value);
}
