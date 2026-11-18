//! Exemplo de plugin: /menu abre uma loja em um menu de baú sem baú, /menugrande abre um menu de
//! 54 slots. Os itens não podem ser tirados; cada clique chega em on_menu_click.

use ceres_plugin::*;

#[derive(Default)]
struct MenuExemplo {
    cliques: u32,
}

fn loja() -> Menu {
    let mut m = Menu::new("loja", "§l§6Loja do Servidor", MenuSize::Chest);
    m.set(11, Item::new("minecraft:diamond").name("§bDiamante").lore(&["§7Clique para ganhar 1 diamante"]));
    m.set(13, Item::new("minecraft:ender_pearl").name("§dTeleportar").lore(&["§7Leva você 10 blocos para cima", "§7(o menu vai junto)"]));
    m.set(15, Item::new("minecraft:barrier").name("§cFechar"));
    m.fill(Item::new("minecraft:black_stained_glass_pane").name(" "));
    m
}

fn menu_grande() -> Menu {
    let mut m = Menu::new("grande", "§l§2Menu Grande", MenuSize::DoubleChest);
    for slot in 0..54 {
        let cor = if slot % 2 == 0 { "minecraft:lime_wool" } else { "minecraft:green_wool" };
        m.set(slot, Item::new(cor).name(&format!("§aSlot {slot}")));
    }
    m.set(49, Item::new("minecraft:barrier").name("§cFechar"));
    m
}

impl Plugin for MenuExemplo {
    fn on_enable(&mut self) {
        register_command("menu", "Abre o menu de exemplo");
        register_command("menugrande", "Abre o menu de exemplo com 54 slots");
        log("MenuExemplo ligado!");
    }

    fn on_disable(&mut self) {
        log("MenuExemplo desligado.");
    }

    fn on_command(&mut self, player: &Player, command: &str, _args: &[String]) {
        match command {
            "menu" => player.open_menu(&loja()),
            "menugrande" => player.open_menu(&menu_grande()),
            _ => {}
        }
    }

    fn on_menu_click(&mut self, player: &Player, menu: &str, slot: u32) {
        self.cliques += 1;
        match (menu, slot) {
            ("loja", 11) => {
                player.give(&Item::new("minecraft:diamond"));
                player.send_message("§aVocê ganhou 1 diamante!");
            }
            ("loja", 13) => {
                player.teleport_by(0.0, 10.0, 0.0);
                player.send_message("§dTeleportado! O menu continua aberto.");
            }
            ("loja", 15) | ("grande", 49) => player.close_menu(),
            ("grande", s) => {
                player.set_menu_item(s, Some(&Item::new("minecraft:gold_block").name(&format!("§6Clicado! ({})", self.cliques))));
            }
            _ => {}
        }
    }

    fn on_menu_close(&mut self, player: &Player, menu: &str) {
        player.send_message(&format!("§7Menu {menu} fechado."));
    }
}

ceres_plugin::plugin!(MenuExemplo);
