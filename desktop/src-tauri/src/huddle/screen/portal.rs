//! Persistent Wayland ScreenCast permission and bounded PipeWire capture.

use ashpd::desktop::{
    screencast::{CursorMode, Screencast, SelectSourcesOptions, SourceType},
    PersistMode, Session,
};
use gstreamer::{self as gst, prelude::*};
use gstreamer_app::AppSink;
use gstreamer_video::{VideoFrameExt, VideoFrameRef, VideoInfo};
use image::RgbaImage;
use std::os::fd::{AsRawFd, OwnedFd};
use std::sync::Arc;
use tokio_util::sync::CancellationToken;

fn capture_pipeline() -> Result<gst::Pipeline, String> {
    // Keep square pixels upstream of videoscale: otherwise PipeWire can
    // fixate its unconstrained aspect ratio to 1/INT_MAX and collapse to 2x2.
    gst::parse::launch(
        "pipewiresrc name=source do-timestamp=true ! video/x-raw,pixel-aspect-ratio=1/1 ! queue max-size-buffers=2 leaky=downstream ! videoconvert ! videoscale ! video/x-raw,format=RGBA,width=[2,1920],height=[2,1080],pixel-aspect-ratio=1/1 ! appsink name=frames max-buffers=1 drop=true sync=false",
    )
    .map_err(|error| error.to_string())?
    .downcast::<gst::Pipeline>()
    .map_err(|_| "Could not construct the screen capture pipeline".into())
}

/// One OS permission grant and one continuous stream for the entire share.
pub struct PortalCapture {
    pipeline: gst::Pipeline,
    sink: AppSink,
    session: Arc<Session<Screencast>>,
    _remote: OwnedFd,
    closed: bool,
}

impl std::fmt::Debug for PortalCapture {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PortalCapture").finish_non_exhaustive()
    }
}

impl PortalCapture {
    /// Open the OS screen/window picker once, retaining its session and remote FD.
    pub async fn open(cancel: &CancellationToken) -> Result<Self, String> {
        let proxy = Screencast::new().await.map_err(|e| e.to_string())?;
        let session = Arc::new(
            proxy
                .create_session(Default::default())
                .await
                .map_err(|e| e.to_string())?,
        );
        let setup = async {
            proxy
                .select_sources(
                    &session,
                    SelectSourcesOptions::default()
                        .set_sources(SourceType::Monitor | SourceType::Window)
                        .set_multiple(false)
                        .set_cursor_mode(CursorMode::Embedded)
                        .set_persist_mode(PersistMode::DoNot),
                )
                .await
                .map_err(|e| e.to_string())?
                .response()
                .map_err(|e| e.to_string())?;
            let response = proxy
                .start(&session, None, Default::default())
                .await
                .map_err(|e| e.to_string())?
                .response()
                .map_err(|e| e.to_string())?;
            let node = response
                .streams()
                .first()
                .ok_or("No screen or window was selected")?
                .pipe_wire_node_id();
            let remote = proxy
                .open_pipe_wire_remote(&session, Default::default())
                .await
                .map_err(|e| e.to_string())?;
            gst::init().map_err(|e| e.to_string())?;
            let pipeline = capture_pipeline()?;
            let source = pipeline
                .by_name("source")
                .ok_or("PipeWire source is unavailable")?;
            source.set_property("fd", remote.as_raw_fd());
            source.set_property("path", node.to_string());
            let sink = pipeline
                .by_name("frames")
                .ok_or("Video capture sink is unavailable")?
                .downcast::<AppSink>()
                .map_err(|_| "Invalid video capture sink")?;
            Ok::<_, String>((pipeline, sink, remote))
        };
        let result = tokio::select! {
            _ = cancel.cancelled() => Err("Screen selection was cancelled".into()),
            result = tokio::time::timeout(std::time::Duration::from_secs(120), setup) =>
                result.map_err(|_| "Screen selection timed out".to_owned()).and_then(|result| result),
        };
        match result {
            Ok((pipeline, sink, remote)) => {
                let capture = Self {
                    pipeline,
                    sink,
                    session,
                    _remote: remote,
                    closed: false,
                };
                if let Err(error) = capture.pipeline.set_state(gst::State::Playing) {
                    capture
                        .close()
                        .await
                        .map_err(|close| format!("{error}; capture cleanup failed: {close}"))?;
                    return Err(error.to_string());
                }
                Ok(capture)
            }
            Err(error) => {
                tokio::time::timeout(std::time::Duration::from_secs(2), session.close())
                    .await
                    .map_err(|_| format!("{error}; closing the screen permission timed out"))?
                    .map_err(|close| {
                        format!("{error}; closing the screen permission failed: {close}")
                    })?;
                Err(error)
            }
        }
    }

    /// Pull the latest bounded raw frame, respecting negotiated row stride.
    pub fn frame(&self) -> Result<Option<RgbaImage>, String> {
        let Some(sample) = self
            .sink
            .try_pull_sample(gst::ClockTime::from_mseconds(100))
        else {
            if let Some(message) = self
                .pipeline
                .bus()
                .and_then(|bus| bus.pop_filtered(&[gst::MessageType::Error, gst::MessageType::Eos]))
            {
                return Err(match message.view() {
                    gst::MessageView::Error(error) => {
                        format!("Screen-cast pipeline failed: {}", error.error())
                    }
                    _ => "The OS screen-cast session ended".into(),
                });
            }
            return Ok(None);
        };
        rgba_frame(&sample).map(Some)
    }

    /// Stop the pipeline and revoke the one persistent OS capture grant.
    pub async fn close(mut self) -> Result<(), String> {
        let stopped = self
            .pipeline
            .set_state(gst::State::Null)
            .map_err(|e| e.to_string());
        let closed = tokio::time::timeout(std::time::Duration::from_secs(2), self.session.close())
            .await
            .map_err(|_| "Screen permission close timed out".to_owned())
            .and_then(|result| result.map_err(|e| e.to_string()));
        self.closed = closed.is_ok();
        stopped.and(closed)
    }
}

fn rgba_frame(sample: &gst::Sample) -> Result<RgbaImage, String> {
    let caps = sample.caps().ok_or("Video sample has no format")?;
    let info = VideoInfo::from_caps(caps).map_err(|e| e.to_string())?;
    if info.format() != gstreamer_video::VideoFormat::Rgba
        || info.width() == 0
        || info.height() == 0
        || u64::from(info.width()) * u64::from(info.height()) > 40_000_000
    {
        return Err("Invalid captured video format or size".into());
    }
    let buffer = sample.buffer().ok_or("Video sample has no pixels")?;
    let frame =
        VideoFrameRef::from_buffer_ref_readable(buffer, &info).map_err(|e| e.to_string())?;
    let plane = frame.plane_data(0).map_err(|e| e.to_string())?;
    // VideoMeta may pad rows beyond the default stride derived from caps.
    let stride = usize::try_from(frame.plane_stride()[0]).map_err(|e| e.to_string())?;
    let row = info.width() as usize * 4;
    if stride < row {
        return Err("Invalid video row stride".into());
    }
    let mut pixels = Vec::with_capacity(row * info.height() as usize);
    for y in 0..info.height() as usize {
        let start = y * stride;
        pixels.extend_from_slice(
            plane
                .get(start..start + row)
                .ok_or("Truncated video frame")?,
        );
    }
    RgbaImage::from_raw(info.width(), info.height(), pixels)
        .ok_or("Invalid captured video pixels".into())
}

impl Drop for PortalCapture {
    fn drop(&mut self) {
        // A panicking worker must not leave a live PipeWire capture behind.
        if let Err(error) = self.pipeline.set_state(gst::State::Null) {
            tracing::error!(%error, "Could not stop the screen capture pipeline");
        }
        if !self.closed {
            let session = Arc::clone(&self.session);
            if let Ok(runtime) = tokio::runtime::Handle::try_current() {
                runtime.spawn(async move {
                    match tokio::time::timeout(std::time::Duration::from_secs(2), session.close())
                        .await
                    {
                        Ok(Ok(())) => {}
                        Ok(Err(error)) => {
                            tracing::error!(%error, "Could not revoke screen capture permission");
                        }
                        Err(error) => {
                            tracing::error!(%error, "Screen capture permission cleanup timed out");
                        }
                    }
                });
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gstreamer_video::{VideoFormat, VideoFrameFlags, VideoMeta};

    #[test]
    fn capture_negotiates_square_pixels_before_scaling_an_ultrawide_source() {
        gst::init().unwrap();
        let pipeline = capture_pipeline().unwrap();
        let source = pipeline.by_name("source").unwrap();
        let source_caps = gst::Caps::builder("video/x-raw")
            .field("format", "BGRA")
            .field("width", 3440_i32)
            .field("height", 1440_i32)
            .build();
        let offered = source
            .static_pad("src")
            .unwrap()
            .peer_query_caps(Some(&source_caps));
        assert!(
            !offered.is_empty(),
            "the native source must remain supported"
        );
        for structure in offered.iter() {
            assert_eq!(
                structure
                    .get::<gst::Fraction>("pixel-aspect-ratio")
                    .unwrap(),
                gst::Fraction::new(1, 1),
                "PipeWire must not fixate an arbitrary videoscale aspect ratio"
            );
        }
    }

    #[test]
    fn capture_reads_actual_padded_rows_without_encoding_padding_as_pixels() {
        gst::init().unwrap();
        let info = VideoInfo::builder(VideoFormat::Rgba, 7, 3).build().unwrap();
        let mut pixels = vec![255; 32 * 3];
        for y in 0..3 {
            pixels[y * 32..y * 32 + 28].fill(y as u8);
        }
        let mut buffer = gst::Buffer::from_mut_slice(pixels);
        VideoMeta::add_full(
            buffer.get_mut().unwrap(),
            VideoFrameFlags::empty(),
            VideoFormat::Rgba,
            7,
            3,
            &[0],
            &[32],
        )
        .unwrap();
        let sample = gst::Sample::builder()
            .buffer(&buffer)
            .caps(&info.to_caps().unwrap())
            .build();
        let image = rgba_frame(&sample).unwrap();
        assert_eq!(image.dimensions(), (7, 3));
        for y in 0..3 {
            assert!(image
                .rows()
                .nth(y)
                .unwrap()
                .all(|pixel| pixel.0 == [y as u8; 4]));
        }
    }
}
