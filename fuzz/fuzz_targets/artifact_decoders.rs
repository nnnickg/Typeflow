#![no_main]

use libfuzzer_sys::fuzz_target;
use typeflow_core::data::{DictionaryIndex, LanguageModel};

fuzz_target!(|data: &[u8]| {
    let _ = LanguageModel::from_artifact_bytes(data);
    let _ = DictionaryIndex::from_artifact_bytes(data);
});
