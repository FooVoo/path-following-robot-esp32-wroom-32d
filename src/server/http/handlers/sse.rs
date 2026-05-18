//! SSE handler — `/events` stream.

use std::convert::Infallible;
use std::time::Duration;

use axum::{
    extract::State,
    response::sse::{Event, KeepAlive, Sse},
};
use tokio_stream::{wrappers::BroadcastStream, StreamExt as _};

use crate::server::{http::state::AppState, ports::event_publisher::EventPublisher};

pub async fn sse_handler(
    State(state): State<AppState>,
) -> Sse<impl tokio_stream::Stream<Item = Result<Event, Infallible>>> {
    let rx = state.events.subscribe();
    let stream = BroadcastStream::new(rx).filter_map(|result| {
        result
            .ok()
            .map(|msg| Ok(Event::default().event("telemetry").data(msg)))
    });
    Sse::new(stream).keep_alive(
        KeepAlive::new()
            .interval(Duration::from_secs(15))
            .text("ping"),
    )
}
