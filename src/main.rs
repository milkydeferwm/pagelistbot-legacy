use std::fs;
use lazy_static::lazy_static;
use apiservice::APIService;
use routine::TaskFinder;
use serde_json::Value;
use tracing::{span, event, Level};
use tracing_subscriber::{fmt::format::FmtSpan, filter, prelude::*};

mod parser;
mod solver;
mod routine;

mod arg;
mod apiservice;
mod types;

lazy_static! {
    static ref API_SERVICE: APIService = APIService::new();
}

/// The main function parses command line arguments, and extracts important information from config files.
/// API_SERVICE is initialized, and TASK_FINDER is fired
#[tokio::main]
async fn main() {
    let args = arg::build_argparse().get_matches();

    // set up subscriber
    let file_appender = tracing_appender::rolling::daily(format!("logs/{}", args.value_of("profile").unwrap()), "plbot.log");
    let (non_blocking, _guard) = tracing_appender::non_blocking(file_appender);
    tracing_subscriber::registry()
    /*
        .with(
            tracing_subscriber::fmt::layer()
                .with_span_events(FmtSpan::NONE)
                .with_filter(filter::LevelFilter::INFO)
        )
    */
        .with(
            tracing_subscriber::fmt::layer()
                .with_writer(non_blocking)
                .with_ansi(false)
                .with_span_events(FmtSpan::NONE)
                .with_filter(filter::LevelFilter::DEBUG)
        )
        .init();

    let (profile, login) = span!(target: "main", Level::INFO, "bootstrap").in_scope(|| {
        event!(Level::INFO, "reading config files");
        event!(Level::DEBUG, "reading site information file");
        let sites = fs::read_to_string(args.value_of("site").unwrap()).expect("cannot open site information file");
        event!(Level::DEBUG, "parsing site information file");
        let sites: Value = serde_json::from_str(&sites).expect("cannot parse site information file");

        let profile = args.value_of("profile").unwrap();
        event!(Level::DEBUG, "fetching profile \"{}\"", profile);
        let profile: types::SiteProfile = serde_json::from_value(sites[profile].clone()).expect("cannot find specified site profile");

        event!(Level::DEBUG, "reading login file");
        let login = fs::read_to_string(args.value_of("login").unwrap()).expect("cannot open login file");
        event!(Level::DEBUG, "parsing login file");
        let login: Value = serde_json::from_str(&login).expect("cannot parse login file.");
        event!(Level::DEBUG, "fetching login credential \"{}\"", &profile.login);
        let login: types::LoginCredential = serde_json::from_value(login[&profile.login].clone()).expect("cannot find specified site profile");

        event!(Level::INFO, "read config files successful");
        (profile, login)
    });

    let config_loc = profile.config.to_owned();

    lazy_static! {
        static ref TASK_FINDER: TaskFinder = TaskFinder::new();
    }

    API_SERVICE.setup(login, profile).await;
    API_SERVICE.try_init().await;
    API_SERVICE.start().await;

    TASK_FINDER.set_config_location(&config_loc).await;
    TASK_FINDER.start().await;

    let ctrl_c_res = tokio::signal::ctrl_c().await;
    match ctrl_c_res {
        Ok(()) => event!(Level::INFO, "ctrl-c detected"),
        Err(err) => event!(Level::ERROR, "unable to listen for shutdown signal: {}", err),
    }

}
