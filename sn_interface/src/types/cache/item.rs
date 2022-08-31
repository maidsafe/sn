// Copyright 2021 MaidSafe.net limited.
//
// This SAFE Network Software is licensed to you under The General Public License (GPL), version 3.
// Unless required by applicable law or agreed to in writing, the SAFE Network Software distributed
// under the GPL Licence is distributed on an "AS IS" BASIS, WITHOUT WARRANTIES OR CONDITIONS OF ANY
// KIND, either express or implied. Please review the Licences for the specific language governing
// permissions and limitations relating to use of the SAFE Network Software.

use std::time::{Duration, Instant};

#[derive(Clone, Debug)]
pub struct Item<T> {
    pub object: T,
    time: Option<Time>,
}

#[derive(Clone, Copy, Debug)]
struct Time {
    start: Instant,
    expiry: Instant,
}

impl<T> Item<T> {
    pub fn new(object: T, item_duration: Option<Duration>) -> Self {
        let time = item_duration.map(|duration| {
            let start = Instant::now();
            Time {
                start,
                expiry: start + duration,
            }
        });
        Item { object, time }
    }

    pub fn expired(&self) -> bool {
        self.time
            .map(|time| time.expiry < Instant::now())
            .unwrap_or(false)
    }

    pub fn elapsed(&self) -> u128 {
        self.time
            .map(|time| Instant::now() - time.start)
            .unwrap_or_default()
            .as_millis()
    }
}

#[cfg(test)]
mod tests {
    use super::Item;
    use std::time::Duration;

    const OBJECT: &str = "OBJECT";

    #[tokio::test(flavor = "multi_thread")]
    async fn not_expired_when_duration_is_none() {
        let item = Item::new(OBJECT, None);
        assert!(!item.expired());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn expired_when_duration_is_zero() {
        let item = Item::new(OBJECT, Some(Duration::new(0, 0)));
        tokio::time::sleep(Duration::new(0, 0)).await;
        assert!(item.expired());
    }
}
