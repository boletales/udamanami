use std::char;

use serenity::{
    builder::{CreateCommand, CreateCommandOption},
    model::application::{CommandOptionType, ResolvedOption, ResolvedValue},
};

enum BrainfuckCommand {
    MoveRight,
    MoveLeft,
    Increment,
    Decrement,
    Output,
    Input,
    LoopStart,
    LoopEnd,
    Invalid,
}

impl From<char> for BrainfuckCommand {
    fn from(c: char) -> Self {
        match c {
            '>' => Self::MoveRight,
            '<' => Self::MoveLeft,
            '+' => Self::Increment,
            '-' => Self::Decrement,
            '.' => Self::Output,
            ',' => Self::Input,
            '[' => Self::LoopStart,
            ']' => Self::LoopEnd,
            _ => Self::Invalid,
        }
    }
}

pub fn register() -> CreateCommand {
    CreateCommand::new("bf")
        .description("Brainfuckを実行するよ")
        .add_option(
            CreateCommandOption::new(CommandOptionType::String, "code", "Brainfuckのコード")
                .required(true),
        )
        .add_option(
            CreateCommandOption::new(CommandOptionType::String, "input", "入力する文字列")
                .required(false),
        )
}

pub fn run(option: &[ResolvedOption]) -> String {
    let ResolvedValue::String(code) = option[0].value else {
        return "エラーだよ！".to_owned();
    };
    let code = code.chars().map(BrainfuckCommand::from).collect::<Vec<_>>();

    let input = match option[1].value {
        ResolvedValue::String(s) => s,
        _ => "",
    };

    let output = interpreter(code, input);
    match output {
        Ok(output) => {
            if output.is_empty() {
                "出力はなかったよ！".to_owned()
            } else {
                output
            }
        }
        Err(err) => {
            format!("エラーだよ！\n```{}```", err)
        }
    }
}

const MAX_LOOP_COUNT: usize = 1024;
const MAX_LOOP_DEPTH: usize = 1024;
const MEMORY_SIZE: usize = 1024;

fn interpreter(code: Vec<BrainfuckCommand>, input: &str) -> Result<String, String> {
    let mut code_pointer = 0;
    let mut memory_pointer = 0;
    let mut memory = vec![0u8; MEMORY_SIZE];
    let mut input_iter = input.chars();
    let mut output = String::new();
    let mut loop_stack = Vec::new();

    while code_pointer < code.len() {
        let cmd = &code[code_pointer];
        match cmd {
            BrainfuckCommand::MoveRight => memory_pointer += 1,
            BrainfuckCommand::MoveLeft => memory_pointer -= 1,
            BrainfuckCommand::Increment => memory[memory_pointer] += 1,
            BrainfuckCommand::Decrement => memory[memory_pointer] -= 1,
            BrainfuckCommand::Output => output.push(memory[memory_pointer] as char),
            BrainfuckCommand::Input => {
                if let Some(input_char) = input_iter.next() {
                    memory[memory_pointer] = if input_char as u16 > 255 {
                        b'?'
                    } else {
                        input_char as u8
                    };
                } else {
                    memory[memory_pointer] = 0;
                }
            }
            BrainfuckCommand::LoopStart => {
                if memory[memory_pointer] == 0 {
                    let mut loop_count = 0;
                    let mut loop_depth = 1;

                    while loop_depth > 0 {
                        loop_count += 1;
                        code_pointer += 1;
                        match code[code_pointer] {
                            BrainfuckCommand::LoopStart => loop_depth += 1,
                            BrainfuckCommand::LoopEnd => loop_depth -= 1,
                            _ => {}
                        }
                        if code_pointer >= code.len() {
                            break;
                        }
                        if loop_depth > MAX_LOOP_DEPTH {
                            return Err("ループが深すぎるよ！".to_owned());
                        }
                        if loop_count > MAX_LOOP_COUNT {
                            return Err("ループが多すぎるよ！　無限ループじゃない？".to_owned());
                        }
                    }
                } else {
                    loop_stack.push(code_pointer);
                }
            }
            BrainfuckCommand::LoopEnd => {
                if memory[memory_pointer] != 0 {
                    code_pointer = *loop_stack.last().unwrap();
                } else {
                    loop_stack.pop();
                }
            }
            BrainfuckCommand::Invalid => {}
        }
        code_pointer += 1;
    }
    Ok(output)
}
