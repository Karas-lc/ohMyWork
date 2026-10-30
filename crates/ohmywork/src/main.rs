fn main() {
    let editor_demo = std::env::args().any(|argument| argument == "--editor-demo");
    ohmywork::app::run(editor_demo);
}
