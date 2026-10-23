use serde::{Deserialize, Serialize};

use crate::Service;

pub const GLOBAL: &str = "global";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Progress {
    Regions(Vec<String>),
    Started {
        region: String,
        service: Service,
    },
    Finished {
        region: String,
        service: Service,
        count: Option<usize>,
        errors: usize,
    },
}
