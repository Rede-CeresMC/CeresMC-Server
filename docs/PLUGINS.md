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
4. No jogo, use `/menu` ou `/menupaginas`.

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
| `on_form(player, form, resposta)` | o jogador respondeu ou fechou um formulário do plugin |
| `on_task(tarefa)` | uma tarefa agendada chegou na hora |

### Eventos canceláveis

Retornam `bool`: `true` deixa acontecer e `false` cancela. Se qualquer plugin cancelar, a ação não acontece.

| Método | O que dá para cancelar |
|---|---|
| `on_chat(player, mensagem)` | a mensagem no chat |
| `on_block_break(player, bloco)` | quebrar o bloco (ele volta para o jogador) |
| `on_block_place(player, bloco)` | colocar o bloco |
| `on_damage(player, dano)` | o dano (queda, fogo, golpe...) |
| `on_attack(atacante, alvo)` | um jogador bater em outro |

`Block` tem `name` (`"minecraft:stone"`), `world`, `x`, `y` e `z`.

### O que o plugin pode fazer

- **Comandos e console:**
  - `register_command(nome, descrição)`: aparece na lista de comandos com autocompletar.
  - `log(texto)`: escreve no console.
  - `broadcast(texto)`: manda uma mensagem para todos.
  - `run_console_command(linha)`: roda um comando como o console.
  - `online_players()`: lista os jogadores online.
- **Jogador (`Player`):**
  - `send_message`, `send_title`, `send_actionbar`;
  - `teleport(x, y, z)`, `teleport_by(dx, dy, dz)`;
  - `give(item)`;
  - `open_menu(menu)`, `set_menu_item(slot, item)`, `close_menu()`;
  - `send_form(id, form)`;
  - `set_scoreboard(título, &[linhas])` e `clear_scoreboard()`: placar na lateral, até 15 linhas, de cima para baixo;
  - `set_bossbar(título, progresso 0.0 a 1.0, cor)` e `clear_bossbar()`.
- **Itens (`Item`):** `Item::new("minecraft:...")` com `.count(n)`, `.meta(n)`, `.name("§a...")` e `.lore(&[...])`.
- **Tarefas** (20 ticks = 1 segundo):
  - `schedule(atraso)`: roda uma vez;
  - `schedule_repeating(atraso, período)`: repete;
  - `cancel_task(id)`: para.

  Cada uma chama `on_task(id)`. Quando o plugin é desligado, as tarefas dele param.

### Formulários

```rust
// botões: a resposta é FormResponse::Button(índice)
player.send_form("loja", &Form::simple("Loja", "Escolha").button("Espadas").button_image("Pocoes", "textures/items/potion_bottle_heal"));
// sim/não: FormResponse::Modal(true = primeiro botão)
player.send_form("confirmar", &Form::modal("Sair", "Tem certeza?", "Sim", "Não"));
// campos: FormResponse::Custom(valores, um por campo na ordem)
player.send_form("perfil", &Form::custom("Perfil").input("Nome", "seu nome", "").toggle("Privado", false).slider("Idade", 10.0, 99.0, 1.0, 18.0).dropdown("Cor", &["Azul", "Verde"], 0));
```

O servidor confere a resposta antes de entregar. Um botão que não existe, um número de campos errado ou o formulário fechado viram `FormResponse::Closed`. Cada formulário só pode ser respondido uma vez.

As ações pedidas pelo plugin são feitas pelo servidor logo depois do evento que o plugin está tratando.

## Menus de baú (InvMenu)

`Menu::new(id, título, MenuSize::Chest)` cria um menu de 27 slots. Por enquanto não há menus de 54: o Minecraft mostra 27 slots para todas as entidades testadas. Para mais itens, use páginas, trocando os slots com `set_menu_item`. A janela continua aberta. O `/menupaginas` do exemplo mostra como.

- **Não existe baú no mundo.** O menu é o inventário de uma entidade invisível, sem modelo, colisão ou nome. Só aquele jogador recebe essa entidade, e ela acompanha o jogador a cada tick. Por isso o menu não fecha por distância, nem voando de élitro.
- **Os itens não saem.** Todo clique é recusado e vira `on_menu_click`. Nada é tirado nem colocado.
- **O menu acompanha o teleporte.** A entidade pula junto com o jogador e a janela continua aberta.
- **O menu fecha sozinho** quando o jogador morre, troca de mundo, sai ou quando o plugin é desligado. Nesses casos o plugin recebe `on_menu_close`.

## Limites de segurança

- **Instruções:** cada evento pode rodar até 200 milhões. Acima disso, o plugin é desligado (protege contra loop infinito).
- **Memória:** até 128 MB por plugin.
- **Encadeamento:** um evento pode gerar até 64 eventos em sequência. Por exemplo, um menu substituído por outro gera um `on_menu_close`.
- **Comandos repetidos:** um comando do servidor sempre vence um comando de plugin com o mesmo nome.
