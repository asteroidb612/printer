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
#[macro_use]
extern crate cfg_if;
#[macro_use]
extern crate lazy_static; // Are singletons evil? elmy? https://stackoverflow.com/questions/27791532/how-do-i-create-a-global-mutable-singleton

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
use rouille::{Response, Server, log};
use serial::prelude::*;
use std::default::Default;
use std::env::var;
use std::fs::File;
use std::io::prelude::*;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use std::sync::mpsc;
use std::io;

cfg_if! {
    if #[cfg(target_os = "macos")] {
        static PRINTER_PATH: &str = "/dev/stdout";
        static PORT: &str = "0.0.0.0:8080";
        static STORAGE: &str = "store.json";
        static TOKEN_STORAGE: &str = "token";
    } else {
        static PRINTER_PATH: &str = "/dev/serial0";
        static PORT: &str = "0.0.0.0:80";
        static STORAGE: &str = "/data/store.json";
        static TOKEN_STORAGE: &str = "/data/token";
    }
}

lazy_static! {
    // Reasons we're touching the model:
    // - To update it with a Msg (handled by update)
    // - To send a serialization of it (handled by view)
    // - To iterate through games while printing (Direct MODEL access, then clone)
    static ref MODEL: Mutex<Model> = {
        let path = Path::new(STORAGE);
        Mutex::new(File::open(&path).and_then(|mut file|{
            let mut s = String::new();
            file.read_to_string(&mut s)?;
            let model : Model = serde_json::from_str(&mut s)?;
            Ok(model)
        }).unwrap_or(Default::default()))
    };
}

fn serialized_view() -> String {
    serde_json::to_string(&view()).expect("Model wasn't serializable").to_owned()
}

fn view() -> Model {
    MODEL.lock().unwrap().to_owned()
}

fn update(msg: Msg) {
    //Scope to prevent deadlock on recursion
    let recursively_update_meta_games;
    { 
        let mut model = MODEL.lock().unwrap();
        let now = Local::now();

        //Update
        match msg {
            //TODO CRIT - THIS PROBABLY DOESN"T WORK
            Msg::GameOccurence(occurrence_name, time) =>{  
                for mut game in model.games.clone() { //TODO Why did I have to add clone here? It was borrowed otherwise? implicitly?
                    if game.name == occurrence_name && game.end > now {
                        game.events.push(time);
                    }
                }
            },
            Msg::GameCreate(name, start, end) => {
                let new_game = Game {
                    name: name,
                    start: start,
                    end: end,
                    events: vec![],
                };
                model.games.push(new_game);
            },
            Msg::Replace(new_model) => {
                let backup_name = format!("/data/backup {} store.json", Local::now().timestamp());
                std::fs::copy(STORAGE, &backup_name).expect("Copying backup store.json failed");
                *model = new_model;
            },
        }

        // Persistence
        serde_json::to_string(&*model)
            .map_err(|serde_err|{ io::Error::new(io::ErrorKind::Other, serde_err)}).and_then(|serialized|{
                let mut file = File::create(Path::new(STORAGE))?;
                file.write_all(serialized.as_bytes())?;
                Ok(())
            }).expect("Writing on update failed");

        //Decide whether to update meta games
        let mut work_on_time = false;
        let mut sleep_on_time = false;
        let mut clenches = false;
        let mut picks = false;
        let mut played_20 = false;

        let today = Local::now().date();
        let weekday = today.weekday();
        //Fixes "cannot move out of borrowed content" https://stackoverflow.com/questions/40862191/cannot-move-out-of-borrowed-content-when-iterating-the-loop
        for game in model.games.clone() {
            let game_from_today = match game.events.into_iter().last() {
                Some(last_game) => last_game.date() == today,
                None => false
            };
            if game_from_today {
                if game.name == "Game_Two" {
                    return //Prevents infinite recursion
                }
                if game.name == "work_on_time" || weekday == Sat || weekday == Sun {
                    work_on_time = true;
                }
                if game.name == "sleep_on_time" || weekday == Sat || weekday == Fri {
                    sleep_on_time = true;
                }
                if game.name == "no_clenches" {
                    clenches = true; 
                }
                if game.name == "no_picks" {
                    picks = true;
                }
                if game.name == "played_20" {
                    played_20 = true;
                }
            }
        }
        recursively_update_meta_games  = work_on_time && sleep_on_time && !clenches && !picks && played_20;
    }

    if recursively_update_meta_games {
        update(Msg::GameOccurence("Game_Two".to_owned(), Local::now()));
    }
}


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

    //Set up a channels for communication between threads
    let (send_msg, recieve_msg) = mpsc::channel();
    let for_web_proc = Arc::new(Mutex::new(send_msg));
    let for_cron = for_web_proc.clone();

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
                            )); //Sometimes this has meant that the printer was off for a while and had to renegotiate keys
                }
            };
        }
        print(format!("{}", view_from_items(all_events)));
        for game in view().games
                .iter()
                .sorted_by(|a, b| Ord::cmp(&b.start, &a.start))
                {
                    if game.name == "Game_Two" {
                        print(github_graph(&game));
                        // Gradually layering games isn't thought through yet
                        //if consecutive_days(game) < 7 {
                        //    break;
                        //}
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
                update(Msg::GameOccurence("ynab".to_owned(), Local::now()));
            }
        };

    //Could be nice to invert this - only spawn the thread if we have the file.
    std::thread::spawn(|| {
        let server = Server::new(PORT, move |request| {
            log(request, io::stdout(), || {

                router!(request,
                        (GET) ["/"] => {
                            let file = File::open("site/index.html").unwrap();
                            Response::from_file("text/html; charset=utf8", file) },
                            (GET) ["/games"] => {
                                Response::text(serialized_view())
                            },
                            (POST) ["/games/{name}/{weeks}", name:String, weeks:i64] => {
                                let start = Local::now();
                                let end = (&start).checked_add_signed(OlderDuration::weeks(weeks)).expect("TimeOverflow");
                                update(Msg::GameCreate(name, start, end));
                                Response::text(serialized_view())
                            },
                            (POST) ["/overwrite_game_file"] => {
                                let new_model: Model  = try_or_400!(rouille::input::json_input(request));
                                update(Msg::Replace(new_model));
                                Response::text(serialized_view())
                            },
                            (GET) ["/read_game_file"] => {
                                let file = File::open(STORAGE).unwrap();
                                Response::from_file("text/html; charset=utf8", file)
                            },
                            (GET) ["/print"] => {
                                let tx1 = for_web_proc.lock().unwrap();
                                tx1.send(0).unwrap();
                                Response::text("Okay".to_owned()) 
                            },
                            (GET) ["/{name}", name: String] => {
                                update(Msg::GameOccurence(name, Local::now()));
                                Response::text(serialized_view())
                            },
                            _ => Response::empty_404()
                                )
            } )
        }).expect("Unable to start web server");
        server.run();
        print("The server stopped running!".to_owned());
    });

    print_next_five_days(); //How is this ownership fine? But not in the cron closure?! I had to convert cron to channels, why not this?
    cron.add(Job::new(
            "0 0 15 * * *".parse().unwrap(), //Package users Greenwhich mean time, so PAC is 15 - 7 == 8:00
            move || {
                let tx1 = for_cron.lock().unwrap();
                tx1.send(0).unwrap();
            }
            ));
    //    cron.add(Job::new("0 0 1/3 0 0 0".parse().unwrap(), check_ynab_api)); //Hours divisible by 3
    loop {
        cron.tick();
        if let Ok(_) = recieve_msg.try_recv() {
            print_next_five_days();
            }
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

    for (key, group) in &sorted_events.iter().group_by(|t| t.0.date()) {
        view.push_str(&format!("{}{}:\n", weekday_name(key.weekday()), key.format("%e")));
        for event in group.into_iter() {
            view.push_str(&format!("  {} {}\n", &event.0.time().format("%H:%M").to_string(), &event.1));
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
    /* Balena has the server always in UTC time
     * It doesn't really matter what timezone the stamps are store in.
     * If we want accurate printing through, we want the days to line up with CA, 
     * I'll have to change this come daylights savings time
     * */
    let now = Local::now();
    let california =  FixedOffset::west(7 * 3600); //TODO Fix for daylight savings
    if now < g.end && now > g.start {
        let dates = &g
            .events
            .iter()
            .map(|d| d.with_timezone(&california).date())
            .collect::<Vec<Date<FixedOffset>>>();
        let today = now.with_timezone(&california).date();
        let last_day = g.end.with_timezone(&california).date();
        let mut day_pointer = match dates.iter().min() {
            Some(x) => x.clone(), //TODO why does clone() change the type here? //(Later) do I see a type error or a borrow error...
            None => g.start.clone().with_timezone(&california).date()
        };
        while day_pointer.weekday() != Sun {
            //Backup to sunday before game starts
            day_pointer = day_pointer.pred()
        }
        let mut output = String::from(format!("Game /{}\n", &g.name));
        while day_pointer <= last_day{
            //Walk through days till today
            if day_pointer.weekday() == Sun {
                output.push_str("\n")
            }
            if dates.contains(&day_pointer) {
                output.push_str("X");
            } else if day_pointer == today {
                output.push_str("O");
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

#[derive(Serialize, Deserialize, Clone, Debug)]
struct Meta {
    name: String,
    start: Time,
    end: Time,
    all: Vec<String>,
    none: Vec<String>
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
struct Model {
    games: Vec<Game>,
    metas: Option<Vec<Meta>>
}

#[derive(Clone)]
enum Msg {
    GameOccurence(String, Time),
    GameCreate(String, Time, Time), //TODO Should this message just carry a game?
    Replace(Model)
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

pub struct PrinterAuthenticatorDelegate;
impl AuthenticatorDelegate for PrinterAuthenticatorDelegate {
    fn present_user_code(&mut self, pi: &PollInformation) {
        print(format!(
                "Please enter {} at {} and grant access to this application",
                pi.user_code, pi.verification_url
                ))
    }
}

fn _consecutive_days(g: &Game) -> i32 {
    let dates = g
        .events
        .iter()
        .map(DateTime::date)
        .collect::<Vec<Date<Local>>>();
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
