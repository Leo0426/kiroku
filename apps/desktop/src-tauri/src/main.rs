// Windows release 下不弹出控制台窗口
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    kiroku_desktop_lib::run()
}
