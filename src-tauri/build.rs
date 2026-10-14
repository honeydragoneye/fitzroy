fn main() {
    // 트레이 아이콘은 컴파일할 때 실행 파일에 들어간다. 파일이 바뀌면 다시 빌드하게 한다.
    println!("cargo:rerun-if-changed=icons/tray.png");
    tauri_build::build()
}
