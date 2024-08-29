use std::{fmt::Display, sync::Arc, time::Duration};

use anyhow::Result;
use clap::{Parser, ValueEnum};
use crossterm::event::{Event, EventStream, KeyCode, KeyEvent};
use futures::StreamExt as _;
use nmea::{sentences::FixType, ParseResult};
use ratatui::{
    layout::{Constraint, Flex, Layout, Rect},
    prelude::Backend,
    text::Text,
    widgets::{Block, Paragraph},
    Frame, Terminal,
};
use tokio::{
    fs::File,
    io::{AsyncBufReadExt, AsyncRead, BufReader},
    sync::RwLock,
    time::Instant,
};

#[derive(Parser, Debug)]
struct Args {
    source: Option<String>,
    #[clap(short, long, default_value_t = Default::default())]
    r#type: SourceType,

    #[clap(long, default_value = "1s")]
    timeout: humantime::Duration,
}

#[derive(ValueEnum, Default, PartialEq, Eq, Clone, Copy, Debug)]
pub enum SourceType {
    #[default]
    File,
    Stdin,
}

impl Display for SourceType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::File => f.write_str("file"),
            Self::Stdin => f.write_str("stdin"),
        }
    }
}

#[tokio::main]
async fn main() {
    let args = Args::parse();

    let nmea = Arc::new(RwLock::new(NmeaStatus::new(args.timeout.into())));

    {
        let nmea = Arc::clone(&nmea);

        let source: Box<dyn AsyncRead + Unpin + Send> = match (args.source, args.r#type) {
            (Some(source), SourceType::File) => {
                Box::new(File::open(source).await.expect("Failed to open file."))
            }
            _ => Box::new(tokio::io::stdin()),
        };

        let mut stdin = BufReader::with_capacity(128, source).lines();

        tokio::spawn(async move {
            loop {
                let line = stdin.next_line().await.unwrap();
                if let Some(line) = line {
                    if let Ok(parsed) = nmea::parse_str(line.trim_end()) {
                        match parsed {
                            ParseResult::GGA(gga) => {
                                let mut nmea = nmea.write().await;
                                nmea.lat.update(gga.latitude);
                                nmea.lon.update(gga.longitude);
                                nmea.alt.update(gga.altitude.map(From::from));
                                nmea.fix_type.update(gga.fix_type.map(|t| match t {
                                    FixType::Invalid => "Invalid",
                                    FixType::Gps => "Gps",
                                    FixType::DGps => "DGps",
                                    FixType::Pps => "Pps",
                                    FixType::Rtk => "Rtk",
                                    FixType::FloatRtk => "FloatRtk",
                                    FixType::Estimated => "Estimated",
                                    FixType::Manual => "Manual",
                                    FixType::Simulation => "Simulation",
                                }));
                            }
                            _ => {}
                        }
                    }
                }
            }
        });
    }

    let terminal = ratatui::init();

    let result = run(terminal, nmea).await;

    ratatui::restore();

    result.expect("Failed to run app.");
}

#[derive(Default, Debug)]
struct NmeaStatus {
    lat: StatusValue<f64>,
    lon: StatusValue<f64>,
    alt: StatusValue<f64>,
    hdg: StatusValue<f64>,
    sog: StatusValue<f64>,
    cog: StatusValue<f64>,
    fix_type: StatusValue<&'static str>,
}

impl NmeaStatus {
    pub fn new(timeout: Duration) -> NmeaStatus {
        NmeaStatus {
            lat: StatusValue::new(timeout),
            lon: StatusValue::new(timeout),
            alt: StatusValue::new(timeout),
            hdg: StatusValue::new(timeout),
            sog: StatusValue::new(timeout),
            cog: StatusValue::new(timeout),
            fix_type: StatusValue::new(timeout),
        }
    }
}

async fn run(mut terminal: Terminal<impl Backend>, nmea: Arc<RwLock<NmeaStatus>>) -> Result<()> {
    let mut interval = tokio::time::interval(Duration::from_secs_f64(1.0 / 60.0));
    let mut events = EventStream::new();

    while tokio::select! {
        _ = interval.tick() => {
            let nmea = nmea.read().await;
            terminal.draw(|frame| draw(frame, &nmea)).expect("Failed to draw terminal.");
            true
        }
        Some(Ok(event)) = events.next() => {
            !matches!(event, Event::Key(KeyEvent { code: KeyCode::Esc, ..}))
        }
    } {}

    Ok(())
}

fn draw(frame: &mut Frame, nmea: &NmeaStatus) {
    let [lat, lon, alt, hdg, sog, cog, fix] = Layout::horizontal([
        Constraint::Length(20), // lat
        Constraint::Length(20), // lon
        Constraint::Length(20), // alt
        Constraint::Length(20), // hdg
        Constraint::Length(20), // sog
        Constraint::Length(20), // cog
        Constraint::Length(20), // status
    ])
    .flex(Flex::Start)
    .areas(frame.area());

    render_statistics(frame, lat, "latitude", nmea.lat.clone());
    render_statistics(frame, lon, "longitude", nmea.lon.clone());
    render_statistics(frame, alt, "altitude", nmea.alt.clone());
    render_statistics(frame, hdg, "heading", nmea.hdg.clone());
    render_statistics(frame, sog, "sog", nmea.sog.clone());
    render_statistics(frame, cog, "cog", nmea.cog.clone());
    render_statistics(frame, fix, "fix", nmea.fix_type.clone());
}

fn render_statistics<'a, T>(frame: &mut Frame, area: Rect, title: &str, value: T)
where
    T: Into<Text<'a>>,
{
    let block = Block::new().title(title);
    frame.render_widget(Paragraph::new(value).block(block), area);
}

#[derive(Clone, Debug)]
struct StatusValue<T> {
    inner: Option<T>,
    updated_at: Instant,
    timeout: Duration,
}

impl<T> Default for StatusValue<T> {
    fn default() -> Self {
        StatusValue {
            inner: None,
            updated_at: Instant::now(),
            timeout: Duration::from_secs(5),
        }
    }
}

impl<T> StatusValue<T> {
    pub fn new(timeout: Duration) -> StatusValue<T> {
        StatusValue {
            timeout,
            ..Default::default()
        }
    }

    pub fn update(&mut self, next: impl Into<Option<T>>) {
        self.inner = next.into();
        self.updated_at = Instant::now();
    }

    pub fn get(&self) -> Option<&T> {
        self.inner
            .as_ref()
            .filter(|_| self.updated_at.elapsed() < self.timeout)
    }
}

impl<T> From<StatusValue<T>> for Text<'_>
where
    T: ToString,
{
    fn from(value: StatusValue<T>) -> Self {
        match value.get() {
            Some(v) => Text::from(v.to_string()),
            None => Text::from("value"),
        }
    }
}
