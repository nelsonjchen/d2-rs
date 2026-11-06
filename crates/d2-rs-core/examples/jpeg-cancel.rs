// SPDX-License-Identifier: MPL-2.0
use d2_rs_core::{
    LayoutError,
    jpeg_entropy::{Error, Input},
};
use serde_json::{Value, json};
use std::{
    cell::{Cell, RefCell},
    io,
};
struct Spy<'a> {
    data: &'a [u8],
    offset: usize,
    events: &'a RefCell<Vec<Value>>,
}
impl Input for Spy<'_> {
    fn read(&mut self, dst: &mut [u8]) -> (usize, Option<Error>) {
        let n = dst.len().min(self.data.len() - self.offset);
        dst[..n].copy_from_slice(&self.data[self.offset..self.offset + n]);
        self.offset += n;
        let error = if n == 0 { Some(Error::Eof) } else { None };
        self.events.borrow_mut().push(json!({"Kind":"read","Poll":0,"Requested":dst.len(),"N":n,"Offset":self.offset,"Error":error.map_or(String::new(),|e|e.to_string())}));
        (n, error)
    }
}
fn main() {
    let inputs: Vec<Value> = serde_json::from_reader(io::stdin().lock()).unwrap();
    let mut outputs = Vec::new();
    for input in inputs {
        let data: Vec<u8> = input["Data"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_u64().unwrap() as u8)
            .collect();
        let at = input["At"].as_u64().unwrap() as usize;
        let deadline = input["Cause"] == "deadline";
        let polls = Cell::new(0usize);
        let events = RefCell::new(Vec::new());
        let poll = || {
            let n = polls.get() + 1;
            polls.set(n);
            let e = if at > 0 && n >= at {
                Some(if deadline {
                    LayoutError::DeadlineExceeded
                } else {
                    LayoutError::Cancelled
                })
            } else {
                None
            };
            events.borrow_mut().push(json!({"Kind":"poll","Poll":n,"Requested":0,"N":0,"Offset":0,"Error":e.as_ref().map_or(String::new(),|e|e.to_string())}));
            e.map_or(Ok(()), Err)
        };
        let mut spy = Spy {
            data: &data,
            offset: 0,
            events: &events,
        };
        let image = d2_rs_core::jpeg_first::decode(&data, &mut spy, &poll);
        let mut out = match image {
            Ok(m) => json!({"Width":m.width,"Height":m.height,"Error":"","Pixels":m.rgba}),
            Err(e) => json!({"Width":0,"Height":0,"Error":e,"Pixels":[]}),
        };
        out["Polls"] = json!(polls.get());
        out["Events"] = json!(events.into_inner());
        outputs.push(out);
    }
    serde_json::to_writer(io::stdout().lock(), &outputs).unwrap();
}
