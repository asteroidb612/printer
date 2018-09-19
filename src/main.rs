extern crate chrono;
extern crate google_calendar3 as calendar3;
extern crate hyper;
extern crate hyper_rustls;
extern crate itertools;
extern crate job_scheduler;
extern crate rouille;
extern crate yup_oauth2 as oauth2;

extern crate serde;
extern crate serde_json;

use job_scheduler::{Job, JobScheduler};

use itertools::Itertools;
use std::fs::File;
use std::io::prelude::*;
use std::path::Path;
use std::time::Duration;

use calendar3::{CalendarHub, Error::*, Event};
use chrono::offset::*;
use chrono::prelude::Local;
use chrono::Datelike;
use chrono::Duration as OlderDuration; //recommended nameing in docs, i think
use chrono::{Date, DateTime, NaiveDate, NaiveDateTime, NaiveTime, Weekday, Weekday::*};
use oauth2::{
    read_application_secret, ApplicationSecret, Authenticator, DefaultAuthenticatorDelegate,
    MemoryStorage,
};
use rouille::Response;
use std::default::Default;
use std::sync::{Arc, Mutex};

#[cfg(target_os = "macos")]
static DEFAULT_PATH: &str = "./output";
#[cfg(target_os = "linux")]
static DEFAULT_PATH: &str = "/dev/serial0";

fn main() {
    let secret: ApplicationSecret =
        read_application_secret(&Path::new("./secret")).expect("Couldn't read application secret");
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
    let hub = Arc::new(Mutex::new(CalendarHub::new(
        hyper::Client::with_connector(hyper::net::HttpsConnector::new(
            hyper_rustls::TlsClient::new(),
        )),
        auth,
    )));
    let hub2 = hub.clone(); //Appease borrow checking gods

    let path = Path::new(DEFAULT_PATH);
    let display = path.display();

    // Open a file in write-only mode, returns `io::Result<File>`
    let mut file = match File::create(&path) {
        Err(why) => panic!("couldn't create file in write-only mode: {}", why),
        Ok(file) => file,
    };

    let mut sched = JobScheduler::new();
    let days = Arc::new(Mutex::new(Vec::new())); //I guess haveing two of these means moving's fine
    let days2 = days.clone();

    let ping_server = || {
        let now = Local::now();
        let next_week = now
            .clone()
            .checked_add_signed(OlderDuration::weeks(1))
            .expect("Time Overflow");

        // You can configure optional parameters by calling the respective setters at will, and
        // execute the final call using `doit()`.
        let _result = hub
            .lock()
            .unwrap()
            .events()
            .list(&"dlazzeri1@gmail.com")
            .time_min(&now.to_rfc3339())
            .time_max(&next_week.to_rfc3339())
            .doit();
    };

    let print_next_five_days = move || {
        let now = Local::now();
        let next_week = now
            .clone()
            .checked_add_signed(OlderDuration::weeks(1))
            .expect("Time Overflow");

        // You can configure optional parameters by calling the respective setters at will, and
        // execute the final call using `doit()`.
        let result = hub2
            .lock()
            .unwrap()
            .events()
            .list(&"dlazzeri1@gmail.com")
            .time_min(&now.to_rfc3339())
            .time_max(&next_week.to_rfc3339())
            .doit();

        match result {
            Err(e) => match e {
                // The Error enum provides details about what exactly happened.
                // You can also just use its `Debug`, `Display` or `Error` traits
                HttpError(_)
                | MissingAPIKey
                | MissingToken(_)
                | Cancelled
                | UploadSizeLimitExceeded(_, _)
                | Failure(_)
                | BadRequest(_)
                | FieldClash(_)
                | JsonDecodeError(_, _) => println!("{}", e),
            },
            Ok((_res, events)) => {
                let u = days2.lock().unwrap();
                let t = u.to_vec();
                let consec = github_graph(t);
                //TODO Docs didn't mention to_vec()? why so many layers?
                let s = format!("\nHabits\n{}\n", consec);
                match file.write_all(&s.as_bytes()) {
                    Err(why) => panic!("couldn't write to printer: {}", why),
                    Ok(_) => println!("successfully wrote to {}", display),
                }

                let string = string_from_items(events.items.expect("No items to parse"));
                match file.write_all(&string.as_bytes()) {
                    Err(why) => panic!("couldn't write to printer: {}", why),
                    Ok(_) => println!("successfully wrote to {}", display),
                }
                println!("\n\n\n{}\n\n\n", string);
            }
        }
    };

    std::thread::spawn(|| {
        rouille::start_server("0.0.0.0:80", move |_request| {
            let mut store = days.into_inner().expect("Corrupted store");
            store.push(Local::now());
            Response::text(serde_json::to_string(&store).unwrap())
        });
    });

    ping_server();
    sched.add(Job::new("0 30 * * * * *".parse().unwrap(), ping_server));
    sched.add(Job::new(
        "0 0 15 * * * *".parse().unwrap(),
        print_next_five_days,
    ));
    loop {
        sched.tick();

        std::thread::sleep(Duration::from_millis(500));
    }
}

fn string_from_items(items: Vec<Event>) -> std::string::String {
    let mut return_string: std::string::String = "".to_string();
    let simplified_events = items.into_iter().map(|event| {
        let start = event.start.expect("No start time for event");
        //TODO I really wanna break out this date logic, but I can't think of what the type interface would be
        let when = match start.date {
            Some(time) => {
                let date = NaiveDate::parse_from_str(&time, "%Y-%m-%d")
                    .expect("Couldn't parse into Naive Date");
                let time = NaiveTime::from_hms(0, 0, 0);
                let date_time = NaiveDateTime::new(date, time);
                FixedOffset::west(7 * 3600)
                    .from_local_datetime(&date_time)
                    .unwrap()
            }
            None => match start.date_time {
                Some(time) => DateTime::parse_from_rfc3339(&time).expect("Couldn't parse dates"),
                None => panic!("There isn't a date or a date_time on event {:?}", start),
            },
        };
        let summary = event.summary.expect("No summary for event");
        (when, summary)
    });

    let sorted_events = simplified_events.sorted_by_key(|t| t.0);

    for (key, group) in &sorted_events.into_iter().group_by(|t| t.0.date().weekday()) {
        return_string.push_str(&format!("{}:\n", weekday_name(key)));
        for event in group.into_iter() {
            //TODO Print time for each event
            //TODO get all-day tasks in line with the day^
            return_string.push_str(&format!("  {}\n", &event.1));
        }
    }
    return_string
}

fn weekday_name(w: Weekday) -> std::string::String {
    let name = match w {
        Mon => "Monday",
        Tue => "Tuesday",
        Wed => "Wednesday",
        Thu => "Thursday",
        Fri => "Friday",
        Sat => "Saturday",
        Sun => "Sunday",
    };
    name.to_owned()
}

fn consecutive_days(v: Vec<DateTime<Local>>) -> i32 {
    let dates = v.iter().map(DateTime::date).collect::<Vec<Date<Local>>>();
    let mut max = 0;
    let dates2 = dates.clone(); //Eww
    for date in dates.into_iter() {
        let mut d = date.pred();
        let mut tmp = 1;
        while dates2.contains(&d) {
            d = d.pred();
            tmp = tmp + 1;
        }
        if max < tmp {
            max = tmp;
        }
    }
    max
}

fn github_graph(v: Vec<DateTime<Local>>) -> String {
    let dates = v.iter().map(DateTime::date).collect::<Vec<Date<Local>>>();
    let today = Local::now().date();
    let min = dates.iter().min().expect("No dates so far").clone(); //TODO why does clone() change the type here?
    let mut date = min.clone();
    while date.weekday() != Sun {
        date = date.pred()
    }
    let mut output = String::from("");
    while date <= today {
        if date.weekday() == Sun {
            output.push_str("\n")
        }
        if dates.contains(&date) {
            output.push_str("X");
        } else {
            output.push_str("_");
        }
        date = date.succ()
    }
    format!("\n{}\n", output)
}
