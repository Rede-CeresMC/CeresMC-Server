# Plugins

Os plugins do CeresMC são escritos em **Rust** e compilados para **WebAssembly**. O servidor roda cada um isolado:

- Um plugin que dá erro, entra em loop infinito ou gasta memória demais é desligado. O servidor continua rodando.
- `/reload` carrega a versão nova sem desligar o servidor.

## Começando

1. Instale o alvo WebAssembly uma vez:

   ```sh
   rustup target add wasm32-unknown-unknown
   ```

2. Copie o exemplo `plugins-exemplo/MenuExemplo` para `plugins/MenuExemplo`.
3. Ligue o servidor. Ele compila o plugin sozinho e mostra `[Plugins] MenuExemplo v1.0.0 enabled`.
4. No jogo, use `/menu` ou `/menugrande`.

Depois de mudar o código, use `/reload`. O servidor recompila em segundo plano e troca os plugins quando termina.

## Comandos

| Comando | Quem pode usar | O que faz |
|---|---|---|
| `/plugins` | todos | lista os plugins (verde = ligado, vermelho = desligado) |
| `/reload` | OP | recompila e recarrega todos os plugins |
| `/plugin enable <nome>` | OP | liga um plugin |
| `/plugin disable <nome>` | OP | desliga um plugin |

Quando um plugin é desligado, os comandos dele somem e os menus que ele abriu fecham.

## Pasta `plugins/`

Formatos aceitos:

- **Pasta com código:** `plugins/Nome/` com `plugin.yml`, `Cargo.toml` e `src/lib.rs`. O servidor compila com `cargo` ao ligar e no `/reload`. A compilação vai para `plugins/.build/`.
- **Pasta já compilada:** `plugins/Nome/` com `plugin.yml` e `plugin.wasm`. É o formato para um servidor de produção sem Rust instalado.
- **Arquivo solto:** `plugins/Nome.wasm`.

`plugin.yml`:

```yaml
name: MenuExemplo
version: 1.0.0
description: O que o plugin faz
```

`Cargo.toml` (o `[workspace]` vazio é necessário porque o plugin fica dentro da pasta do servidor):

```toml
[package]
name = "meu-plugin"
version = "1.0.0"
edition = "2024"

[lib]
crate-type = ["cdylib"]

[dependencies]
ceres-plugin = { path = "../../crates/ceres-plugin" }

[workspace]
```

## Escrevendo um plugin

```rust
use ceres_plugin::*;

#[derive(Default)]
struct MeuPlugin;

impl Plugin for MeuPlugin {
    fn on_enable(&mut self) {
        register_command("loja", "Abre a loja");
    }

    fn on_command(&mut self, player: &Player, command: &str, _args: &[String]) {
        if command == "loja" {
            let mut menu = Menu::new("loja", "§6Loja", MenuSize::Chest);
            menu.set(13, Item::new("minecraft:diamond").name("§bDiamante").lore(&["§7Clique para comprar"]));
            player.open_menu(&menu);
        }
    }

    fn on_menu_click(&mut self, player: &Player, menu: &str, slot: u32) {
        if menu == "loja" && slot == 13 {
            player.give(&Item::new("minecraft:diamond"));
            player.close_menu();
        }
    }
}

ceres_plugin::plugin!(MeuPlugin);
```

### Eventos (`trait Plugin`)

| Método | Quando é chamado |
|---|---|
| `on_enable` | o plugin foi carregado; registre os comandos aqui |
| `on_disable` | `/reload`, `/plugin disable` ou o servidor desligando |
| `on_join(player)` | um jogador entrou (também os que já estavam online quando o plugin ligou) |
| `on_quit(player)` | um jogador saiu |
| `on_command(player, comando, args)` | um jogador usou um comando do plugin |
| `on_menu_click(player, menu, slot)` | clique num slot de um menu do plugin |
| `on_menu_close(player, menu)` | o menu fechou (pelo jogador ou pelo servidor) |

### O que o plugin pode fazer

- **Comandos e console:**
  - `register_command(nome, descrição)`: aparece na lista de comandos com autocompletar.
  - `log(texto)`: escreve no console.
  - `broadcast(texto)`: manda uma mensagem para todos.
  - `run_console_command(linha)`: roda um comando como o console.
  - `online_players()`: lista os jogadores online.
- **Jogador (`Player`):**
  - `send_message`, `send_title`;
  - `teleport(x, y, z)`, `teleport_by(dx, dy, dz)`;
  - `give(item)`;
  - `open_menu(menu)`, `set_menu_item(slot, item)`, `close_menu()`.
- **Itens (`Item`):** `Item::new("minecraft:...")` com `.count(n)`, `.meta(n)`, `.name("§a...")` e `.lore(&[...])`.

As ações pedidas pelo plugin são feitas pelo servidor logo depois do evento que o plugin está tratando.

## Menus de baú (InvMenu)

`Menu::new(id, título, MenuSize::Chest)` cria um menu de 27 slots. `MenuSize::DoubleChest` cria um de 54.

- **Ninguém mais vê o baú.** O servidor mostra um baú só para aquele jogador, dois blocos abaixo dos pés, dentro do chão. Quando o menu fecha, o bloco verdadeiro volta.
- **Os itens não saem.** Todo clique é recusado e vira `on_menu_click`. Nada é tirado nem colocado.
- **O menu acompanha o teleporte.** Se o jogador for teleportado com o menu aberto, ele reabre no destino com o mesmo conteúdo.
- **O menu fecha sozinho** quando o jogador morre, troca de mundo, sai ou quando o plugin é desligado. Nesses casos o plugin recebe `on_menu_close`.

## Limites de segurança

- **Instruções:** cada evento pode rodar até 200 milhões. Acima disso, o plugin é desligado (protege contra loop infinito).
- **Memória:** até 128 MB por plugin.
- **Encadeamento:** um evento pode gerar até 64 eventos em sequência. Por exemplo, um menu substituído por outro gera um `on_menu_close`.
- **Comandos repetidos:** um comando do servidor sempre vence um comando de plugin com o mesmo nome.
