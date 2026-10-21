#![no_main]
libfuzzer_sys::fuzz_target!(|data: &[u8]| olive_html::fuzzing::network_message(data));
