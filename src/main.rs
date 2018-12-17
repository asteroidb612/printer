extern crate chrono;
extern crate google_calendar3 as calendar3;
extern crate hyper;
extern crate hyper_rustls;
extern crate itertools;
extern crate job_scheduler;
extern crate reqwest;
extern crate serde;
extern crate serde_json;
extern crate serial;
extern crate yup_oauth2 as oauth2;

#[macro_use]
extern crate serde_derive;
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
    DiskTokenStorage, PollInformation,
};
use rouille::Response;
use serial::prelude::*;
use std::default::Default;
use std::env::var;
use std::fs::File;
use std::io::prelude::*;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Duration;

#[cfg(target_os = "linux")]
static PRINTER_PATH: &str = "/dev/serial0";
#[cfg(target_os = "linux")]
static PORT: &str = "0.0.0.0:80";
#[cfg(target_os = "linux")]
static STORAGE: &str = "/data/store.json";
#[cfg(target_os = "linux")]
static TOKEN_STORAGE: &str = "/data/token";

#[cfg(target_os = "macos")]
static PRINTER_PATH: &str = "/dev/stdout";
#[cfg(target_os = "macos")]
static PORT: &str = "0.0.0.0:8080";
#[cfg(target_os = "macos")]
static STORAGE: &str = "store.json";
#[cfg(target_os = "macos")]
static TOKEN_STORAGE: &str = "token";

fn print(s: String) {
    let mut write_to = match File::create(Path::new(PRINTER_PATH)) {
        //I have a lot of good case studies in here for moving/borrowing... this is one...
        Err(why) => panic!("couldn't create file in write-only mode: {}", why),
        Ok(mut file) => file,
    };

    let formatted = format!("\n{}\n", s);
    write_to
        .write_all(formatted.as_bytes())
        .expect("Unable to print");
}

fn secret() -> oauth2::ApplicationSecret {
    if cfg!(target_os = "linux") {
        let mut secret_file = File::create("./secret").expect("Couldn't create file for secret");
        let google_oauth_json =
            var("google_oauth_json").expect("Couldn't find google_oauth_json env var");
        secret_file
            .write_all(google_oauth_json.as_bytes())
            .expect("Coudln't write secret to file");
    }

    let secret: ApplicationSecret =
        read_application_secret(&Path::new("./secret")).expect("Couldn't read application secret");
    secret
}

fn setup() {
    if cfg!(target_os = "linux") {
        let mut port = serial::open(Path::new("/dev/serial0")).expect("Couldn't open /dev/serial0");
        port.reconfigure(&mut |settings| settings.set_baud_rate(serial::Baud19200))
            .expect("Couldn't set baudrate");
        port.write("Fuck Accordions".as_bytes())
            .expect("Couldn't write to serial port");
    }
}

fn main() {
    setup();
    print(String::from("It's working... It's working!"));

    let token_storage =
        DiskTokenStorage::new(&TOKEN_STORAGE.to_string()).expect("Couldn't use disk token storage");
    let auth = Authenticator::new(
        &secret(),
        PrinterAuthenticatorDelegate,
        hyper::Client::with_connector(hyper::net::HttpsConnector::new(
            hyper_rustls::TlsClient::new(),
        )),
        token_storage,
        None,
    );
    let hub = CalendarHub::new(
        hyper::Client::with_connector(hyper::net::HttpsConnector::new(
            hyper_rustls::TlsClient::new(),
        )),
        auth,
    );

    let mut cron = JobScheduler::new();
    let path = Path::new(STORAGE);

    //TODO Clean up: This should have one error, one default call, and be flatter
    let model: Model = match File::open(&path) {
        Err(_why) => {
            print!("Couldn't open {:#?}: {}\nContinuing.\n", path, _why);
            Default::default()
        }
        Ok(mut file) => {
            let mut s = String::new();
            match file.read_to_string(&mut s) {
                Err(_why) => {
                    print!("Couldn't read file {:#?}: {}\nContinuing.\n", path, _why);
                    Default::default()
                }
                Ok(_) => match serde_json::from_str(&mut s) {
                    Err(_why) => {
                        print!("Couldn't parse {:#?}: {}\nContinuing.\n", path, _why);
                        Default::default()
                    }
                    Ok(parsed_store) => parsed_store,
                },
            }
        }
    };
    let share_for_web_interface = Arc::new(Mutex::new(model)); //I guess haveing two of these means moving's fine
    let share_for_cron = share_for_web_interface.clone(); //What are memory implications of a move?
    let share_for_ynab = share_for_web_interface.clone();

    let print_next_five_days = move || {
        println!("print_next_five_days");
        let now = Local::now();
        let next_week = now
            .clone()
            .checked_add_signed(OlderDuration::weeks(1))
            .expect("Time Overflow");

        let calendars = ["dlazzeri1@gmail.com", "drew@interviewing.io"];
        let mut all_events = vec![];

        for calendar in calendars.iter() {
            let result = hub
                .events()
                .list(&calendar)
                .single_events(true)
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
                    print(format!(
                        "Unable to connect to Google when looking for calendar {}: {:?}",
                        calendar, x
                    ));
                }
            };
        }
        let model = share_for_cron.lock().unwrap();
        print(format!("{}", view_from_items(all_events)));
        for game in model.games.iter().sorted_by(|a, b| Ord::cmp(&b.start, &a.start)) {
            print(github_graph(&game));
            if consecutive_days(game) < 7 {
                break
            }
        }
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

    //Could be nice to invert this - only spawn the thread if we have the file.
    std::thread::spawn(|| {
        rouille::start_server(PORT, move |request| {
            router!(request,
            (GET) ["/"] => {
                let file = File::open("site/index.html").unwrap();
                Response::from_file("text/html; charset=utf8", file)
            },
            (GET) ["/games"] => {
                let store = share_for_web_interface.lock().unwrap();
                let serialized = serde_json::to_string(&store.clone()).unwrap();
                Response::text(serialized)
            },
            (GET) ["/games/{name}", name: String] => {
                 let store = share_for_web_interface.lock().unwrap();
                 let mut games:Vec<&Game> = store.games.iter().filter(|g| g.name == name).collect(); //TODO Make impossible states impossible
                 match games.pop(){
                     Some(game) => {
                         let serialized = serde_json::to_string(game).unwrap();
                         Response::text(serialized)
                         },
                     None => Response::empty_404()
                 }
            },
            (POST) ["/games/{name}/{weeks}", name:String, weeks:i64] => {
                let mut store = share_for_web_interface.lock().unwrap();
                let start = Local::now();
                let end = (&start) .checked_add_signed(OlderDuration::weeks(weeks)).expect("TimeOverflow");
                *store = updated(&mut *store, Msg::GameCreate(name, start, end));
                let serialized = serde_json::to_string(&store.clone()).unwrap();

                let path = Path::new(STORAGE);
                let mut file = match File::create(path) {
                    Err(_) => panic!("couldn't create file for server storage"),
                    Ok(file) => file
                };
                match file.write_all(serialized.as_bytes()) {
                    Err(_) => panic!("server couldn't write store to file"),
                    Ok(_) => ()
                };
                Response::text(serialized)
            },
            (GET) ["/{name}", name: String] => {
                let mut store = share_for_web_interface.lock().unwrap();
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
                *store = updated(&mut *store, Msg::GameOccurence(name, Local::now()));
                let serialized = serde_json::to_string(&store.clone()).unwrap();

                let path = Path::new(STORAGE);
                let mut file = match File::create(path) {
                    Err(_) => panic!("couldn't create file for server storage"),
                    Ok(file) => file
                };
                match file.write_all(serialized.as_bytes()) {
                    Err(_) => panic!("server couldn't write store to file"),
                    Ok(_) => ()
                };
                Response::text(serialized)
            },
            _ => Response::empty_404()
         )
        });
    });

    print_next_five_days();
    cron.add(Job::new(
        "0 0 15 * * *".parse().unwrap(), //Package users Greenwhich mean time, so PAC is 15 - 7 == 8:00
        print_next_five_days,
    ));
    //    cron.add(Job::new("0 0 1/3 0 0 0".parse().unwrap(), check_ynab_api)); //Hours divisible by 3
    loop {
        cron.tick();

        std::thread::sleep(Duration::from_millis(500));
    }
}

fn view_from_items(items: Vec<calendar3::Event>) -> View {
    let mut view = "".to_string();

    let sorted_events = items
        .iter()
        .filter_map(|i| i.start.iter().zip(&i.summary).next())
        .map(|(start, summary)| {
            //Get sensible date representation
            let when = match &start.date {
                Some(time) => {
                    let date = NaiveDate::parse_from_str(&time, "%Y-%m-%d")
                        .expect("Couldn't parse into Naive Date");
                    let time = NaiveTime::from_hms(0, 0, 0);
                    let date_time = NaiveDateTime::new(date, time);
                    FixedOffset::west(7 * 3600)
                        .from_local_datetime(&date_time)
                        .unwrap()
                }
                None => match &start.date_time {
                    Some(time) => {
                        DateTime::parse_from_rfc3339(&time).expect("Couldn't parse dates")
                    }
                    None => panic!("There isn't a date or a date_time on event {:?}", start),
                },
            };
            (when, summary)
        }).sorted_by_key(|t| t.0); //And sort

    for (key, group) in &sorted_events.iter().group_by(|t| t.0.date().weekday()) {
        view.push_str(&format!("{}:\n", weekday_name(key)));
        for event in group.into_iter() {
            //TODO Print time for each event
            //TODO get all-day tasks in line with the day^
            view.push_str(&format!("  {}\n", &event.1));
        }
    }
    view
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

fn github_graph(g: &Game) -> View {
    let now = Local::now();
    if now < g.end && now > g.start {
        let dates = &g
            .events
            .iter()
            .map(DateTime::date)
            .collect::<Vec<Date<Local>>>();
        let today = now.date();
        let mut day_pointer = match dates.iter().min() {
            Some(x) => x.clone(), //TODO why does clone() change the type here? //(Later) do I see a type error or a borrow error...
            None => {
                return String::from("No Dates For Game So Far");
            }
        };
        while day_pointer.weekday() != Sun {
            //Backup to sunday before game starts
            day_pointer = day_pointer.pred()
        }
        let mut output = String::from(format!("Game /{}\n", &g.name));
        while day_pointer <= today {
            //Walk through days till today
            if day_pointer.weekday() == Sun {
                output.push_str("\n")
            }
            if dates.contains(&day_pointer) {
                output.push_str("X");
            } else {
                output.push_str("_");
            }
            day_pointer = day_pointer.succ()
        }
        format!("\n{}\n", output)
    } else {
        "".to_string()
    }
}

type View = std::string::String;
type Time = chrono::DateTime<Local>;

#[derive(Serialize, Deserialize, Clone, Debug)]
struct Game {
    name: String,
    start: Time,
    end: Time,
    events: Vec<Time>,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
struct Model {
    games: Vec<Game>,
}

enum Msg {
    GameOccurence(String, Time),
    GameCreate(String, Time, Time), //TODO Should this message just carry a game?
}

#[derive(Deserialize, Debug, Default)]
struct YNABResponse {
    data: Data,
}

#[derive(Deserialize, Debug, Default)]
struct Data {
    transactions: Vec<Transaction>,
}

#[derive(Deserialize, Debug, Default)]
struct Transaction {
    approved: bool,
    flag_color: Option<String>,
}

fn updated(model: &mut Model, msg: Msg) -> Model {
    let mut c = model.clone(); //Really? I Have to borrow mut AND clone? Could I just clone? What problems is each solving??
    let now = Local::now();
    match msg {
        Msg::GameOccurence(game, time) => Model {
            games: c
                .games
                .into_iter()
                .map(|mut stored_game| {
                    println!("{:?} stored, {:?} sent", stored_game, game);
                    if stored_game.name == game && stored_game.end > now {
                        stored_game.events.push(time);
                        stored_game //Hey that's not immutable... maybe I miss conslists
                    } else {
                        stored_game
                    }
                }).collect(),
        },
        Msg::GameCreate(name, start, end) => {
            let new_game = Game {
                name: name,
                start: start,
                end: end,
                events: vec![],
            };
            c.games.push(new_game);
            c
        }
    }
}

pub struct PrinterAuthenticatorDelegate;
impl AuthenticatorDelegate for PrinterAuthenticatorDelegate {
    fn present_user_code(&mut self, pi: &PollInformation) {
        print(format!(
            "Please enter {} at {} and grant access to this application",
            pi.user_code, pi.verification_url
        ))
    }
}

fn consecutive_days(g: &Game) -> i32 {
    let dates = g.events.iter().map(DateTime::date).collect::<Vec<Date<Local>>>();
    let mut max = 0;
    for date in (&dates).into_iter() {
        let mut previous = date.pred();
        let mut days = 1;
        while (&dates).contains(&previous) {
            previous = previous.pred();
            days = days + 1;
        }
        if max < days {
            max = days;
        }
    }
    max
}
