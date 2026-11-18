//! Plugin used by tools/plugin_api_test.py: exercises forms, scoreboard, bossbar, tasks and the
//! cancellable events.

use ceres_plugin::*;

#[derive(Default)]
struct Teste {
    task: u32,
    runs: u32,
}

impl Plugin for Teste {
    fn on_enable(&mut self) {
        register_command("formteste", "Teste de formulário");
        register_command("formcampos", "Teste de formulário com campos");
        register_command("placar", "Teste de scoreboard e bossbar");
        register_command("tarefa", "Teste de tarefa repetida");
    }

    fn on_command(&mut self, player: &Player, command: &str, _args: &[String]) {
        match command {
            "formteste" => player.send_form("botoes", &Form::simple("Teste", "Escolha").button("Zero").button("Um")),
            "formcampos" => player.send_form(
                "campos",
                &Form::custom("Campos").input("Nome", "seu nome", "").toggle("Ligado", false).slider("Valor", 0.0, 10.0, 1.0, 5.0),
            ),
            "placar" => {
                player.set_scoreboard("§6Ceres", &["Linha um", "Linha dois", "Linha um"]);
                player.set_bossbar("§cChefe", 0.5, BossBarColor::Red);
                player.send_actionbar("§aBarra");
            }
            "tarefa" => {
                self.runs = 0;
                self.task = schedule_repeating(10, 10);
            }
            _ => {}
        }
    }

    fn on_task(&mut self, task: u32) {
        if task == self.task {
            self.runs += 1;
            broadcast(&format!("tarefa {}", self.runs));
            if self.runs == 2 {
                cancel_task(task);
            }
        }
    }

    fn on_form(&mut self, player: &Player, form: &str, response: FormResponse) {
        let text = match response {
            FormResponse::Closed => "fechado".to_string(),
            FormResponse::Button(i) => format!("botao {i}"),
            FormResponse::Modal(b) => format!("modal {b}"),
            FormResponse::Custom(v) => format!("campos {}", Value::Array(v)),
        };
        player.send_message(&format!("form {form}: {text}"));
    }

    fn on_chat(&mut self, _player: &Player, message: &str) -> bool {
        !message.contains("proibido")
    }

    fn on_block_break(&mut self, _player: &Player, block: &Block) -> bool {
        block.name != "minecraft:gold_block"
    }

    fn on_block_place(&mut self, _player: &Player, block: &Block) -> bool {
        block.name != "minecraft:diamond_block"
    }
}

ceres_plugin::plugin!(Teste);
