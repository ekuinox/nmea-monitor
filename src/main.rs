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
};

#[derive(Parser, Debug)]
struct Args {
    source: Option<String>,
    #[clap(short, long, default_value_t = Default::default())]
    r#type: SourceType,
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

    let nmea = Arc::new(RwLock::new(NmeaStatus::default()));

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
                                nmea.lat = gga.latitude;
                                nmea.lon = gga.longitude;
                                nmea.alt = gga.altitude.map(From::from);
                                nmea.fix_type = gga.fix_type.map(|t| match t {
                                    FixType::Invalid => "Invalid",
                                    FixType::Gps => "Gps",
                                    FixType::DGps => "DGps",
                                    FixType::Pps => "Pps",
                                    FixType::Rtk => "Rtk",
                                    FixType::FloatRtk => "FloatRtk",
                                    FixType::Estimated => "Estimated",
                                    FixType::Manual => "Manual",
                                    FixType::Simulation => "Simulation",
                                });
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
    lat: Option<f64>,
    lon: Option<f64>,
    alt: Option<f64>,
    hdg: Option<f64>,
    sog: Option<f64>,
    cog: Option<f64>,
    fix_type: Option<&'static str>,
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

    render_statistics(
        frame,
        lat,
        "latitude",
        nmea.lat.as_ref().map(ToString::to_string),
    );
    render_statistics(
        frame,
        lon,
        "longitude",
        nmea.lon.as_ref().map(ToString::to_string),
    );
    render_statistics(
        frame,
        alt,
        "altitude",
        nmea.alt.as_ref().map(ToString::to_string),
    );
    render_statistics(
        frame,
        hdg,
        "heading",
        nmea.hdg.as_ref().map(ToString::to_string),
    );
    render_statistics(
        frame,
        sog,
        "sog",
        nmea.sog.as_ref().map(ToString::to_string),
    );
    render_statistics(
        frame,
        cog,
        "cog",
        nmea.cog.as_ref().map(ToString::to_string),
    );
    render_statistics(frame, fix, "fix", nmea.fix_type.map(ToString::to_string));
}

fn render_statistics<'a, T>(frame: &mut Frame, area: Rect, title: &str, value: Option<T>)
where
    T: Into<Text<'a>>,
{
    let block = Block::new().title(title);
    if let Some(value) = value {
        frame.render_widget(Paragraph::new(value).block(block), area);
    } else {
        frame.render_widget(Paragraph::new("null").block(block), area);
    }
}
