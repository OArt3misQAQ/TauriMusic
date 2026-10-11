// Prevents additional console window on Windows in release, DO NOT REMOVE!!
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

// mimalloc 全局分配器:曲库/JSON 场景大量小分配,碎片率低于系统堆,常驻内存更省
#[global_allocator]
static GLOBAL_ALLOCATOR: mimalloc::MiMalloc = mimalloc::MiMalloc;

fn main() {
    tauri_music_lib::run()
}
