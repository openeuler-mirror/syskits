use ctpipeline::value::CtValue;
use ctplugin::proto::{HostFrame, PROTOCOL_VERSION, PluginFrame};
use std::io::{self, BufRead, Write};

fn main() {
    let stdin = io::stdin();
    let stdout = io::stdout();
    let mut reader = stdin.lock();
    let mut writer = stdout.lock();

    // 1. Read Hello
    let mut line = String::new();
    if reader.read_line(&mut line).is_err() || line.is_empty() {
        return;
    }
    let req: HostFrame = serde_json::from_str(&line).unwrap();

    if let HostFrame::Hello { protocol } = req {
        if protocol != PROTOCOL_VERSION {
            return;
        }
    } else {
        return;
    }

    // 2. Send Hello (commands are returned via Signature frame)
    let resp = PluginFrame::Hello {
        protocol: PROTOCOL_VERSION.to_string(),
        commands: vec![],
    };
    writeln!(writer, "{}", serde_json::to_string(&resp).unwrap()).unwrap();

    // 3. Handle Signature / Run
    let mut prefix = String::new();
    loop {
        line.clear();
        if reader.read_line(&mut line).is_err() || line.is_empty() {
            return;
        }
        let frame: HostFrame = serde_json::from_str(&line).unwrap();
        match frame {
            HostFrame::Signature => {
                let sig = PluginFrame::Signature {
                    commands: vec!["plugin-echo".to_string()],
                };
                writeln!(writer, "{}", serde_json::to_string(&sig).unwrap()).unwrap();
            }
            HostFrame::Run { name, args } | HostFrame::Call { name, args } => {
                if name != "plugin-echo" {
                    let err = PluginFrame::Error {
                        message: "unknown command".into(),
                        code: 1,
                    };
                    writeln!(writer, "{}", serde_json::to_string(&err).unwrap()).unwrap();
                    return;
                }
                if let Some(first_arg) = args.positionals.first() {
                    if let CtValue::String(s) = &first_arg.value {
                        prefix = s.clone();
                    } else {
                        prefix = format!("{:?}", first_arg.value);
                    }
                }
                let ack = PluginFrame::CallResponse {
                    accepted: true,
                    message: None,
                    code: 0,
                };
                writeln!(writer, "{}", serde_json::to_string(&ack).unwrap()).unwrap();
                break;
            }
            HostFrame::Goodbye => return,
            _ => {}
        }
    }

    // 4. Read optional input stream and stop at End
    let mut input_count = 0usize;
    loop {
        line.clear();
        if reader.read_line(&mut line).is_err() || line.is_empty() {
            break;
        }
        let frame: HostFrame = match serde_json::from_str(&line) {
            Ok(f) => f,
            Err(_) => break,
        };
        match frame {
            HostFrame::Data { value: _ } => {
                input_count += 1;
            }
            HostFrame::End => break,
            _ => {}
        }
    }

    // 5. Send Data
    let suffix = if input_count > 0 {
        format!(" ({} input frame(s))", input_count)
    } else {
        String::new()
    };
    let data = PluginFrame::Data {
        value: CtValue::String(format!("{} from plugin!{}", prefix, suffix)),
    };
    writeln!(writer, "{}", serde_json::to_string(&data).unwrap()).unwrap();

    let end = PluginFrame::End;
    writeln!(writer, "{}", serde_json::to_string(&end).unwrap()).unwrap();
    let goodbye = PluginFrame::Goodbye;
    writeln!(writer, "{}", serde_json::to_string(&goodbye).unwrap()).unwrap();
}
