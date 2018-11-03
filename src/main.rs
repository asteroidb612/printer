extern crate chrono;
extern crate google_calendar3 as calendar3;
extern crate hyper;
extern crate hyper_rustls;
extern crate itertools;
extern crate job_scheduler;
extern crate reqwest;
extern crate serial;
extern crate yup_oauth2 as oauth2;

#[macro_use]
extern crate serde_derive;
extern crate serde;
extern crate serde_json;
#[macro_use]
extern crate rouille;

use calendar3::CalendarHub;
use chrono::offset::*;
use chrono::prelude::Local;
use chrono::Datelike;
use chrono::Duration as OlderDuration; //recommended nameing in docs, i think
use chrono::{Date, DateTime, NaiveDate, NaiveDateTime, NaiveTime, Weekday, Weekday::*};
use itertools::Itertools;
use job_scheduler::{Job, JobScheduler};
use oauth2::{
    read_application_secret, ApplicationSecret, Authenticator, AuthenticatorDelegate,
    MemoryStorage, PollInformation,
};
use rouille::Response;
use serial::prelude::*;
use serial::SystemPort;
use std::default::Default;
use std::env::var;
use std::fs::File;
use std::io::prelude::*;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Duration;

#[cfg(target_os = "macos")]
static DEFAULT_PATH: &str = "./output";
#[cfg(target_os = "linux")]
static DEFAULT_PATH: &str = "/dev/serial0";

fn main() {
    let mut port = serial::open(Path::new("/dev/serial0")).expect("Couldn't open /dev/serial0");
    port.reconfigure(&mut |settings| settings.set_baud_rate(serial::Baud19200))
        .expect("Couldn't set baudrate");
    port.write("Fuck Accordions".as_bytes())
        .expect("Couldn't write to serial port");

    {
        let mut secret_file = File::create("/secret").expect("Couldn't create file");
        let google_oauth_json =
            var("google_oauth_json").expect("Couldn't find google_oauth_json env var");
        secret_file
            .write_all(google_oauth_json.as_bytes())
            .expect("Coudln't write secret to file");
    }

    let secret: ApplicationSecret =
        read_application_secret(&Path::new("/secret")).expect("Couldn't read application secret");
    let auth = Authenticator::new(
        &secret,
        PrinterAuthenticatorDelegate,
        hyper::Client::with_connector(hyper::net::HttpsConnector::new(
            hyper_rustls::TlsClient::new(),
        )),
        <MemoryStorage as Default>::default(), //TODO Is this not real memory storage? Could I get rid of Ping with this?
        None,
    );
    let hub = Arc::new(Mutex::new(CalendarHub::new(
        hyper::Client::with_connector(hyper::net::HttpsConnector::new(
            hyper_rustls::TlsClient::new(),
        )),
        auth,
    )));
    let hub2 = hub.clone(); //Appease borrow checking gods

    // Open a file in write-only mode, returns `io::Result<File>`
    let mut printer = match File::create(Path::new(DEFAULT_PATH)) {
        //I have a lot of good case studies in here for moving/borrowing... this is one...
        Err(why) => panic!("couldn't create file in write-only mode: {}", why),
        Ok(file) => file,
    };
    let mut print = |s: String| {
        let formatted = format!("\n{}\n", s);
        printer.write_all(formatted.as_bytes()).expect("Unable to print");
    };
    print(String::from("It's working... It's working!"));

    let mut cron = JobScheduler::new();
    let path = Path::new("store.json");
    let model = Model {
        games: match File::open(&path) {
            Err(_why) => vec![],
            Ok(mut file) => {
                let mut s = String::new();
                match file.read_to_string(&mut s) {
                    Err(_why) => vec![],
                    Ok(_) => match serde_json::from_str(&mut s) {
                        Err(_why) => vec![],
                        Ok(parsed_store) => parsed_store,
                    },
                }
            }
        },
    };
    let share_for_web_interface = Arc::new(Mutex::new(model)); //I guess haveing two of these means moving's fine
    let share_for_cron = share_for_web_interface.clone(); //What are memory implications of a move?
    let share_for_ynab = share_for_web_interface.clone();

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

    let mut print_next_five_days = move || {
        let now = Local::now();
        let next_week = now
            .clone()
            .checked_add_signed(OlderDuration::weeks(1))
            .expect("Time Overflow");

        let calendars = ["dlazzeri1@gmail.com", "drew@interviewing.io", "fb Calendar"];
        let mut all_events = vec![];
        // You can configure optional parameters by calling the respective setters at will, and
        // execute the final call using `doit()`.
        for calendar in calendars.iter() {
            let result = hub2
                .lock()
                .unwrap()
                .events()
                .list(&calendar)
                .time_min(&now.to_rfc3339())
                .time_max(&next_week.to_rfc3339())
                .doit();

            match result {
                Ok((_res, events)) => {
                    match events.items {
                        Some(mut e) => {
                            all_events.append(&mut e);
                        }
                        None => {
                            print(format!("No events on calendar {}", calendar));
                        }
                    };
                }
                x => {
                    print(format!("\n\nUnable to connect to Google: {:?} ", x));
                }
            };
        }
        let u = share_for_cron.lock().unwrap();
        let t = u.games.to_vec();
        match t.get(0) {
            Some(x) => {
                let consec = github_graph(&x);
                //TODO Docs didn't mention to_vec()? why so many layers?
                print(format!("\nHabits\n{}\n", consec));
            }
            None => print(format!("No games to log: {:?}", &u)),
        };

        print(format!("{}", string_from_items(all_events)));
    };

    let _check_ynab_api =
        move || {
            let client = reqwest::Client::new();
            let url = "";
            let bear = ""; //move to file
            let mut res = client
                .get(url)
                .bearer_auth(bear)
                .send()
                .expect("Couldn't send request to YNAB");

            let response: YNABResponse = res.json().expect("Couldn't parse respone from YNAB");
            if response.data.transactions.into_iter().all(|transaction| {
                match transaction.flag_color {
                    None => transaction.approved,
                    _ => true,
                }
            }) {
                let mut model = share_for_ynab.lock().expect("Couldn't unlock YNAB share");
                *model = updated(
                    &mut *model,
                    Msg::GameOccurence("ynab".to_owned(), Local::now()),
                );
            }
        };

    std::thread::spawn(|| {
        rouille::start_server("0.0.0.0:80", move |request| {
            router!(request,
            (GET) (/) => {
                let file = File::open("site/index.html").expect("Couldn't find index.html");
                Response::from_file("text/html; charset=utf8", file)
            },
            _ => {
                let mut store = share_for_web_interface.lock().expect("Couldn't find Rouille share");
                //I want a mutable borrow, not a move
                // Can you pass a mutable borrow to functions?
                // Can you set an immutable borrow?
                // What IIIS dereferencing?
                // Is it just taking the shells of of types?
                // Is each layer maybe borrowed, maybe owned?
                // How do I write to a layer? mutating the layer above?
                // &(this) is a place expression. So any place can go there.
                // &(functionCall) is using a temporary, implicit let expression to store functionCall then make the borrow
                // *dereference ALWAYS implicitly borrows!
                // So it never moves
                // So we could tell it to borrow mut or not
                // *share.lock().unwrap() -> *&share.lock().unwrap() -> let &x = &share.lock().unwrap(); *&x
                // Wtf is the place in a place_expression for a borrow? in this article's first example?
                //* -> & -> let
                // ONLY SOMETIMES!
                // f
                //https://stackoverflow.com/questions/51335679/where-is-a-mutexguard-if-i-never-assign-it-to-a-variable
                *store = updated(&mut *store, Msg::GameOccurence(request.url(), Local::now()));
                let serialized = serde_json::to_string(&store.clone()).unwrap();

                let path = Path::new("./store.json");
                let mut file = match File::create(path) {
                    Err(_) => panic!("couldn't create file for server storage"),
                    Ok(file) => file
                };
                match file.write_all(serialized.as_bytes()) {
                    Err(_) => panic!("server couldn't write store to file"),
                    Ok(_) => ()
                };
                Response::text(serialized)
            })
        });
    });

    ping_server();
    print_next_five_days();
    cron.add(Job::new("0 30 * * * *".parse().unwrap(), ping_server));
    cron.add(Job::new(
        "0 0 15 * * *".parse().unwrap(), //Package users Greenwhich mean time, so PAC is 15 - 7 == 8:00
        print_next_five_days,
    ));
    //    cron.add(Job::new("0 0 1/3 0 0 0".parse().unwrap(), YPGR-KGLMcheck_ynab_api)); //Hours divisible by 3
    loop {
        cron.tick();

        std::thread::sleep(Duration::from_millis(500));
    }
}

fn string_from_items(items: Vec<calendar3::Event>) -> std::string::String {
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

fn _consecutive_days(v: Vec<DateTime<Local>>) -> i32 {
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

fn github_graph(g: &Game) -> String {
    let dates = &g
        .events
        .iter()
        .map(|l| DateTime::date(&l.when))
        .collect::<Vec<Date<Local>>>();
    let today = Local::now().date();
    let min = match dates.iter().min() {
        Some(x) => x.clone(), //TODO why does clone() change the type here? //(Later) do I see a type error or a borrow error...
        None => {
            return String::from("No Dates For Game So Far");
        }
    };
    let mut date = min.clone();
    while date.weekday() != Sun {
        date = date.pred()
    }
    let mut output = String::from(format!("Game {}\n", &g.name));
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

#[derive(Serialize, Deserialize, Clone, Debug)]
struct Event {
    what: String,
    when: chrono::DateTime<Local>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
struct Game {
    name: String,
    //start: chrono::DateTime<Local>,
    //end: chrono::DateTime<Local>,
    events: Vec<Event>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
struct Model {
    games: Vec<Game>,
}

enum Msg {
    GameOccurence(String, chrono::DateTime<Local>),
}

#[derive(Deserialize, Debug)]
struct YNABResponse {
    data: Data,
}

#[derive(Deserialize, Debug)]
struct Data {
    transactions: Vec<Transaction>,
}

#[derive(Deserialize, Debug)]
struct Transaction {
    approved: bool,
    flag_color: Option<String>,
}

fn updated(model: &mut Model, msg: Msg) -> Model {
    let c = model.clone(); //Really? I Have to borrow mut AND clone? Could I just clone? What problems is each solving??
    match msg {
        Msg::GameOccurence(game, time) => Model {
            games: c
                .games
                .into_iter()
                .map(|mut stored_game| {
                    if stored_game.name == game {
                        stored_game.events.push(Event {
                            what: game.clone(),
                            when: time,
                        });
                        stored_game //Hey that's not immutable... maybe I miss conslists
                    } else {
                        stored_game
                    }
                })
                .collect(),
        },
    }
}

pub struct PrinterAuthenticatorDelegate;
impl AuthenticatorDelegate for PrinterAuthenticatorDelegate {
    fn present_user_code(&mut self, pi: &PollInformation) {
        let mut port = serial::open(Path::new("/dev/serial0"))
            .expect("PrinterAuthenticatorDelegate couldn't open /dev/serial0");
        port.reconfigure(&mut |settings| settings.set_baud_rate(serial::Baud19200))
            .expect("PrinterAuthenticatorDelegate Couldn't set baudrate");
        port.write(
            format!(
                "Please enter {} at {} and grant access to this application",
                pi.user_code, pi.verification_url
            )
            .as_bytes(),
        )
        .expect("PrinterAuthenticatorDelegate Couldn't write to serial port");
    }
}
