use std::time::Duration;

use chrono::{DateTime, Utc};
use notify::Event;
use tokio::sync::mpsc::Receiver;
use tokio::time::{sleep, Instant};

use crate::util;

pub struct Batch {
    pub events: Vec<Event>,
    pub started_at: DateTime<Utc>,
    pub ended_at: DateTime<Utc>,
}

pub async fn next_batch(rx: &mut Receiver<Event>, window: Duration) -> Option<Batch> {
    let first_event = rx.recv().await?;
    let mut events = vec![first_event];
    let started_at = util::now_utc();
    let deadline = sleep(window);
    tokio::pin!(deadline);
    loop {
        tokio::select! {
            _ = &mut deadline => {
                break;
            }
            maybe_event = rx.recv() => {
                match maybe_event {
                    Some(event) => {
                        events.push(event);
                        let next = Instant::now() + window;
                        deadline.as_mut().reset(next);
                    }
                    None => {
                        break;
                    }
                }
            }
        }
    }
    Some(Batch {
        events,
        started_at,
        ended_at: util::now_utc(),
    })
}
