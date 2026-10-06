use buzz_audio_client::{
    encoder::{AudioEncoder, FRAME_SAMPLES},
    jitter::PeerJitterBuffer,
    wire::{parse_relay_frame, FrameHeader, V2_HEADER_LEN},
};

#[test]
fn encoded_media_plays_through_the_shared_receiver() {
    let mut encoder = AudioEncoder::new().unwrap();
    let mut receiver = PeerJitterBuffer::new(7).unwrap();
    let samples: Vec<f32> = (0..FRAME_SAMPLES)
        .map(|n| (n as f32 * 0.1).sin() * 0.1)
        .collect();
    for seq in 0..8 {
        let payload = encoder.encode(&samples).unwrap();
        let frame = [vec![7], payload].concat();
        let (peer, header, opus) = parse_relay_frame(&frame).unwrap();
        assert_eq!(peer, 7);
        assert_eq!(header.seq, seq);
        assert_eq!(header.ts_48k, u32::from(seq) * 960);
        receiver
            .insert_packet(header.seq, header.ts_48k, opus)
            .unwrap();
    }
    let mut energy = 0.0_f32;
    for _ in 0..20 {
        let (pcm, _) = receiver.get_audio().unwrap();
        assert_eq!(pcm.len(), 480);
        assert!(pcm.iter().all(|s| s.is_finite()));
        energy += pcm.iter().map(|s| s * s).sum::<f32>();
    }
    assert!(
        energy > 0.01,
        "voice must survive actual Opus and jitter decoding"
    );
}

#[test]
fn invalid_input_does_not_advance_clock_and_short_frames_are_padded() {
    let mut encoder = AudioEncoder::new().unwrap();
    for samples in [vec![], vec![0.; 961], vec![f32::NAN], vec![f32::INFINITY]] {
        assert!(encoder.encode(&samples).is_err());
    }
    let encoded = encoder.encode(&[0.1; 120]).unwrap();
    let (header, _) = FrameHeader::parse(&encoded).unwrap();
    assert_eq!(header.seq, 0);
    assert_eq!(header.ts_48k, 0);
    let mut decoder = opus::Decoder::new(48000, opus::Channels::Mono).unwrap();
    let mut pcm = [0.0; 5760];
    assert_eq!(
        decoder
            .decode_float(&encoded[V2_HEADER_LEN..], &mut pcm, false)
            .unwrap(),
        960
    );
}
