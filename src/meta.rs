use time;

use std::io::{self, Read};
use std::collections::HashMap;
use std::num;

use hyper;
use hyper::client::Client;

#[derive(Debug, Clone)]
pub struct Meta {
    pub ctime: u64,
    pub mtime: u64,
    pub finish_time: Option<u64>,
    pub gps: (f32, f32),
    pub tags: HashMap<String, String>,
}

impl Default for Meta {
    fn default() -> Meta {
        Meta {
            ctime: time::get_time().sec as u64,
            mtime: 0,
            finish_time: None,
            gps: gps_query().unwrap_or_else(|e| {
                error!("failed to get gps: {:?}", e);
                (0.0, 0.0)
            }),
            tags: HashMap::new(),
        }
    }
}

impl Meta {
    pub fn bump_mtime(&mut self) {
        self.mtime = time::get_time().sec as u64;
    }
}

fn gps_query() -> Result<(f32, f32), GpsError> {
    let client = Client::new();
    let mut res = try!(client.get("http://ipinfo.io/loc").send());
    let mut text_res = String::new();
    try!(res.read_to_string(&mut text_res));
    let parts = text_res.trim().split(',').collect::<Vec<_>>();

    if parts.len() == 2 {
        let lat = try!(parts[0].parse::<f32>());
        let lon = try!(parts[1].parse::<f32>());
        Ok((lat, lon))
    } else {
        let err_string = format!("unable to parse response: {:?}", text_res);
        let error = GpsError::Other(err_string);
        Err(error)
    }
}

#[derive(Debug)]
enum GpsError {
    Hyper(hyper::Error),
    Io(io::Error),
    Parse(num::ParseFloatError),
    Other(String),
}

impl From<hyper::Error> for GpsError {
    fn from(err: hyper::Error) -> GpsError {
        GpsError::Hyper(err)
    }
}


impl From<io::Error> for GpsError {
    fn from(err: io::Error) -> GpsError {
        GpsError::Io(err)
    }
}

impl From<num::ParseFloatError> for GpsError {
    fn from(err: num::ParseFloatError) -> GpsError {
        GpsError::Parse(err)
    }
}
