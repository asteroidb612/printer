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
use std::env::var;
use std::fs::File;
use std::io::prelude::*;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use std::sync::mpsc;
use std::io;
use std::process::Command;



// Globals


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




// Update


fn update(msg: Msg) {
    let mut model = MODEL.lock().unwrap();
    let now = Local::now();
    match msg {
        Msg::GameOccurence(occurrence_name, time) =>{  
            let games = &mut model.games;
            for ref mut game in games {  //We want ref to games, not the iterator
                if game.name == occurrence_name && game.end > now {
                    let events = &mut game.events;
                    events.push(time);
                }
            }
        },
        Msg::GameCreate(name, start, end) => {
            let new_game = Game {
                name: name,
                start: start,
                end: end,
                events: vec![],
                part_of: None,
                skipping: None 
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
    serde_json::to_string(&*model) //&* because rust doesn't auto reference, only auto deref
        .map_err(|serde_err|{ io::Error::new(io::ErrorKind::Other, serde_err)}).and_then(|serialized|{
            let mut file = File::create(Path::new(STORAGE))?;
            file.write_all(serialized.as_bytes())?;
            Ok(())
        }).expect("Writing on update failed");
}


fn print(s: String) {
    let mut write_to = match File::create(Path::new(PRINTER_PATH)) {
        //I have a lot of good case studies in here for moving/borrowing... this is one...
        Err(why) => panic!("couldn't create file in write-only mode: {}", why),
        Ok(mut file) => file,
    };

    let formatted = format!("\n{}\n\n\n", s); //3 at end so receipt rips nicely
    write_to
        .write_all(formatted.as_bytes())
        .expect("Unable to print");
}



// Main


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
        match current_meta_game() {
            Some(game) => {print(github_graph(&game))},
            None => {}
        };
        print(format!("Gerard Manley Hopkins (1844–89).  Poems.  1918.
 
13. Pied Beauty
 
 
GLORY be to God for dappled things—	
  For skies of couple-colour as a brinded cow;	
    For rose-moles all in stipple upon trout that swim;	
Fresh-firecoal chestnut-falls; finches' wings;	
  Landscape plotted and pieced—fold, fallow, and plough;	        5
    And áll trádes, their gear and tackle and trim.	
 
All things counter, original, spare, strange;	
  Whatever is fickle, freckled (who knows how?)	
    With swift, slow; sweet, sour; adazzle, dim;	
He fathers-forth whose beauty is past change:	        10
                  Praise him."));
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
    try_print_moxie();
    cron.add(Job::new(
            "0 0 12 * * *".parse().unwrap(), //Package users Greenwhich mean time. w/ dst should be 6:00?
            move || {
                let tx1 = for_cron.lock().unwrap();
                tx1.send(0).unwrap();
            }
            ));
    cron.add(Job::new(
            "0 0 2 * * *".parse().unwrap(),
            try_print_moxie
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



// View


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
    let california =  FixedOffset::west(7 * 3600);  // Fix for daylight savings
    if now < g.end && now > g.start {
        let dates = &g
            .events
            .iter()
            .map(|d| d.with_timezone(&california).date())
            .collect::<Vec<Date<FixedOffset>>>();
        let today = now.with_timezone(&california).date();
        let first_day= g.start.with_timezone(&california).date();
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
            } else if day_pointer == first_day || day_pointer == last_day {
                output.push_str("o");
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

fn current_meta_game() -> Option<Game> {
    let model1 = view().to_owned();

    let model2 = model1.clone();
    let metas = model1.metas.unwrap_or(vec![]);
    let name = match metas.first() {
        Some(meta) => meta.name.clone(),
        None => "".to_owned()
    };

    let games : Vec<Game> = model2.games.into_iter().filter(|g| match g.part_of {
        Some(ref game_name) => game_name.clone() == name,
        None => false
       }).collect();

    let all_events: Vec<Time> = games.iter().flat_map(|x| x.events.clone()).collect(); //Pretty sloppy decisions whether to clone and who...

    let california =  FixedOffset::west(7 * 3600);  // Fix for daylight savings
    let valid_events: Vec<Time> = all_events.into_iter().filter(|potential_meta_event| {
        games.iter().all(|game| {
            let day_off = game.clone().skipping.unwrap_or(vec![]).into_iter()
                .find(|skip| potential_meta_event.clone().with_timezone(&california).date().weekday() == skip.clone())
                .is_some();
            let all_games_won = game.events.iter()
                .find(|game_event| {
                    let cali_game_event = game_event.clone().with_timezone(&california).date();
                    let cali_meta_event = potential_meta_event.clone().with_timezone(&california).date();
                    cali_game_event == cali_meta_event})
                .is_some();
        day_off || all_games_won
        })
    }).collect();

    let earliest =  games.iter().min_by(|x, y| x.start.cmp(&y.start));
    let latest =  games.iter().max_by(|x, y| x.end.cmp(&y.end));
    match (earliest, latest) {
        (Some(early), Some(late)) => Some(Game {
            name: name,
            start: early.start,
            end: late.end,
            events: valid_events,
            part_of: None,
            skipping: None, 
        }),
        _ => None
    }
}



// Data Structures


type View = std::string::String;
type Time = chrono::DateTime<Local>;

#[derive(Serialize, Deserialize, Clone, Debug)]
struct Game {
    name: String,
    start: Time,
    end: Time,
    events: Vec<Time>,
    part_of: Option<String>, // Proposed replacement for meta game
    skipping: Option<Vec<Weekday>>
}

#[derive(Serialize, Deserialize, Clone, Debug)]
struct Meta {
    name: String,
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



// Graveyard


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

fn try_print_moxie() {
    match Command::new("./query.sh").output(){
        Ok(s) => {print(String::from_utf8(s.stdout).unwrap())}
        Err(e) => {println!("Error trying to fetch moxie: {:?}", e)}
    }
}
