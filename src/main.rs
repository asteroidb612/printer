extern crate chrono;
extern crate google_calendar3 as calendar3;
extern crate hyper;
extern crate hyper_rustls;
extern crate job_scheduler;
extern crate yup_oauth2 as oauth2;

use job_scheduler::{Job, JobScheduler};

use std::fs::File;
use std::io::prelude::*;
use std::path::Path;
use std::time::Duration;

use calendar3::CalendarHub;
use calendar3::Error;
use chrono::prelude::Local;
use chrono::Duration as OlderDuration; //recommended nameing in docs, i think
use oauth2::{
    read_application_secret, ApplicationSecret, Authenticator, DefaultAuthenticatorDelegate,
    MemoryStorage,
};
use std::default::Default;

fn main() {
    let secret: ApplicationSecret =
        read_application_secret(&Path::new("./secret")).expect("Couldn't read it");
    // Instantiate the authenticator. It will choose a suitable authentication flow for you,
    // unless you replace  `None` with the desired Flow.
    // Provide your own `AuthenticatorDelegate` to adjust the way it operates and get feedback about
    // what's going on. You probably want to bring in your own `TokenStorage` to persist tokens and
    // retrieve them from storage.
    let auth = Authenticator::new(
        &secret,
        DefaultAuthenticatorDelegate,
        hyper::Client::with_connector(hyper::net::HttpsConnector::new(
            hyper_rustls::TlsClient::new(),
        )),
        <MemoryStorage as Default>::default(),
        None,
    );
    let hub = CalendarHub::new(
        hyper::Client::with_connector(hyper::net::HttpsConnector::new(
            hyper_rustls::TlsClient::new(),
        )),
        auth,
    );

    let path = Path::new("/dev/serial0");
    let display = path.display();

    // Open a file in write-only mode, returns `io::Result<File>`
    let mut file = match File::create(&path) {
        Err(why) => panic!("couldn't create file in write-only mode"),
        Ok(file) => file,
    };

    let mut sched = JobScheduler::new();

    sched.add(Job::new("0 0 8 * * *".parse().unwrap(), || {
        let now = Local::now();
        let next_week = now
            .clone()
            .checked_add_signed(OlderDuration::weeks(1))
            .expect("Time Overflow");

        // You can configure optional parameters by calling the respective setters at will, and
        // execute the final call using `doit()`.
        let result = hub
            .events()
            .list(&"dlazzeri1@gmail.com")
            .time_min(&now.to_rfc3339())
            .time_max(&next_week.to_rfc3339())
            .doit();

        match result {
            Err(e) => match e {
                // The Error enum provides details about what exactly happened.
                // You can also just use its `Debug`, `Display` or `Error` traits
                Error::HttpError(_)
                | Error::MissingAPIKey
                | Error::MissingToken(_)
                | Error::Cancelled
                | Error::UploadSizeLimitExceeded(_, _)
                | Error::Failure(_)
                | Error::BadRequest(_)
                | Error::FieldClash(_)
                | Error::JsonDecodeError(_, _) => println!("{}", e),
            },
            Ok((_res, events)) => {
                let items = events.items.expect("No items to parse");
                for item in items.into_iter() {
                    // Write the `LOREM_IPSUM` string to `file`, returns `io::Result<()>`
                    let string = format!(
                        "\n\n{:?} starts at {:?}\n\n",
                        item.summary.expect("No summary for event"),
                        item.start.expect("No start time for event")
                    );
                    match file.write_all(&string.as_bytes()) {
                        Err(why) => panic!("couldn't write to printer"),
                        Ok(_) => println!("successfully wrote to {}", display),
                    }
                    println!();
                }
            }
        }
    }));

    loop {
        sched.tick();

        std::thread::sleep(Duration::from_millis(500));
    }
}
