//! Exemplo de plugin: /menu abre uma loja em um menu de baú sem baú, /menupaginas abre um menu
//! com várias páginas. Os itens não podem ser tirados; cada clique chega em on_menu_click.

use ceres_plugin::*;

#[derive(Default)]
struct MenuExemplo {
    /// página aberta por jogador
    paginas: std::collections::HashMap<String, u32>,
}

fn loja() -> Menu {
    let mut m = Menu::new("loja", "§l§6Loja do Servidor", MenuSize::Chest);
    m.set(11, Item::new("minecraft:diamond").name("§bDiamante").lore(&["§7Clique para ganhar 1 diamante"]));
    m.set(13, Item::new("minecraft:ender_pearl").name("§dTeleportar").lore(&["§7Leva você 10 blocos para cima", "§7(o menu vai junto)"]));
    m.set(15, Item::new("minecraft:barrier").name("§cFechar"));
    m.fill(Item::new("minecraft:black_stained_glass_pane").name(" "));
    m
}

const TOTAL_ITENS: u32 = 40;

/// Os 27 slots de uma página: 18 itens, setas para trocar e o botão de fechar.
fn pagina(numero: u32) -> Vec<(u32, Option<Item>)> {
    let paginas = TOTAL_ITENS.div_ceil(18);
    (0..27)
        .map(|slot| {
            let n = numero * 18 + slot;
            let item = match slot {
                0..=17 if n < TOTAL_ITENS => {
                    let cor = if n % 2 == 0 { "minecraft:lime_wool" } else { "minecraft:green_wool" };
                    Some(Item::new(cor).name(&format!("§aItem {}", n + 1)))
                }
                18 if numero > 0 => Some(Item::new("minecraft:arrow").name("§ePágina anterior")),
                26 if numero + 1 < paginas => Some(Item::new("minecraft:arrow").name("§ePróxima página")),
                22 => Some(Item::new("minecraft:paper").name(&format!("§fPágina {} de {paginas}", numero + 1))),
                23 => Some(Item::new("minecraft:barrier").name("§cFechar")),
                _ => None,
            };
            (slot, item)
        })
        .collect()
}

fn menu_paginas() -> Menu {
    let mut m = Menu::new("paginas", "§l§2Itens", MenuSize::Chest);
    for (slot, item) in pagina(0) {
        if let Some(item) = item {
            m.set(slot, item);
        }
    }
    m
}

impl Plugin for MenuExemplo {
    fn on_enable(&mut self) {
        register_command("menu", "Abre o menu de exemplo");
        register_command("menupaginas", "Abre um menu com várias páginas");
        log("MenuExemplo ligado!");
    }

    fn on_disable(&mut self) {
        log("MenuExemplo desligado.");
    }

    fn on_command(&mut self, player: &Player, command: &str, _args: &[String]) {
        match command {
            "menu" => player.open_menu(&loja()),
            "menupaginas" => {
                self.paginas.insert(player.name.clone(), 0);
                player.open_menu(&menu_paginas());
            }
            _ => {}
        }
    }

    fn on_menu_click(&mut self, player: &Player, menu: &str, slot: u32) {
        match (menu, slot) {
            ("loja", 11) => {
                player.give(&Item::new("minecraft:diamond"));
                player.send_message("§aVocê ganhou 1 diamante!");
            }
            ("loja", 13) => {
                player.teleport_by(0.0, 10.0, 0.0);
                player.send_message("§dTeleportado! O menu continua aberto.");
            }
            ("loja", 15) | ("paginas", 23) => player.close_menu(),
            ("paginas", 18 | 26) => {
                let atual = self.paginas.entry(player.name.clone()).or_insert(0);
                let ultima = TOTAL_ITENS.div_ceil(18) - 1;
                *atual = if slot == 26 { (*atual + 1).min(ultima) } else { atual.saturating_sub(1) };
                // troca só os slots: a janela continua aberta
                for (s, item) in pagina(*atual) {
                    player.set_menu_item(s, item.as_ref());
                }
            }
            ("paginas", s) if s < 18 => {
                player.set_menu_item(s, Some(&Item::new("minecraft:gold_block").name("§6Clicado!")));
            }
            _ => {}
        }
    }

    fn on_menu_close(&mut self, player: &Player, menu: &str) {
        player.send_message(&format!("§7Menu {menu} fechado."));
    }
}

ceres_plugin::plugin!(MenuExemplo);
