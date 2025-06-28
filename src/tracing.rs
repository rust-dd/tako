use tracing::level_filters::LevelFilter;
use tracing_subscriber::{
    Layer, fmt::format::FmtSpan, layer::SubscriberExt, util::SubscriberInitExt,
};

pub fn init_tracing() {
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::fmt::layer()
                .with_span_events(FmtSpan::CLOSE)
                .with_file(true)
                .with_line_number(true)
                .with_level(true)
                .with_filter(LevelFilter::DEBUG),
        )
        .init();
}
