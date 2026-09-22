use super::*;
use std::{collections::HashMap, sync::Mutex, time::Instant};
use tracing::instrument::WithSubscriber;
use tracing_subscriber::{layer::Context, prelude::*, registry::LookupSpan, Layer};

#[derive(Default)]
struct Costs {
    active: Mutex<HashMap<tracing::Id, (&'static str, Instant)>>,
    elapsed: Mutex<Vec<(&'static str, f64)>>,
}
struct Timing(Arc<Costs>);
impl<S> Layer<S> for Timing
where
    S: tracing::Subscriber + for<'a> LookupSpan<'a>,
{
    fn on_new_span(
        &self,
        attrs: &tracing::span::Attributes<'_>,
        id: &tracing::Id,
        _: Context<'_, S>,
    ) {
        let name = attrs.metadata().name();
        if [
            "get_thread_window",
            "thread_window_aux",
            "get_accessible_channel_ids",
        ]
        .contains(&name)
        {
            self.0
                .active
                .lock()
                .unwrap()
                .insert(id.clone(), (name, Instant::now()));
        }
    }
    fn on_close(&self, id: tracing::Id, _: Context<'_, S>) {
        if let Some((name, start)) = self.0.active.lock().unwrap().remove(&id) {
            self.0
                .elapsed
                .lock()
                .unwrap()
                .push((name, start.elapsed().as_secs_f64() * 1000.0));
        }
    }
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn thread_window_cost_evidence_through_signed_router() {
    let f = Fixture::new().await;
    for n in 0..51 {
        f.reply(n).await;
    }
    let aux = f.aux(40003, &f.root, Some(f.channel)).await;
    sqlx::query("INSERT INTO events (community_id,id,pubkey,created_at,kind,tags,content,sig,received_at,channel_id) \
        SELECT community_id,decode(md5(n::text)||md5(('cost'||n)::text),'hex'),pubkey,created_at,kind,tags,repeat('x',500),sig,received_at,channel_id \
        FROM events CROSS JOIN generate_series(1,1000) n WHERE community_id=$1 AND id=$2")
        .bind(f.community.as_uuid()).bind(aux.id.to_bytes().to_vec()).execute(&f.pool).await.unwrap();
    let costs = Arc::new(Costs::default());
    let subscriber = tracing_subscriber::registry().with(Timing(costs.clone()));
    let recorder = metrics_util::debugging::DebuggingRecorder::new();
    let snapshotter = recorder.snapshotter();
    let _guard = metrics::set_default_local_recorder(&recorder);
    let start = Instant::now();
    let (status, body) = f
        .post(&f.keys, "/query", json!([f.filter()]))
        .with_subscriber(subscriber)
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body.as_array().unwrap().len(), 1052);
    assert_eq!(f.bounds(&body, &f.filter())["has_more"], true);
    println!(
        "COST router_ms={} serialized_response_bytes={}",
        start.elapsed().as_secs_f64() * 1000.0,
        body.to_string().len()
    );
    for (name, ms) in costs.elapsed.lock().unwrap().iter() {
        println!("COST span={name} ms={ms}");
    }
    for (key, _, _, value) in snapshotter.snapshot().into_vec() {
        if matches!(
            key.key().name(),
            "buzz_db_pool_acquire_duration_seconds" | "buzz_thread_window_response_bytes"
        ) {
            println!("COST metric={:?} value={value:?}", key.key());
        }
    }
}
