# Roadmap

O objetivo é um servidor Bedrock em Rust com comportamento vanilla e uma API de plugins no estilo PocketMine.
Cada item é pequeno o bastante para ser feito, testado e commitado sozinho.

**Legenda:** `[x]` feito · `[~]` parcial · `[ ]` a fazer
**Prioridade:** 🔴 bloqueia jogar · 🟠 importante · 🟢 desejável

As fases são uma ordem sugerida, não uma regra. Itens de fases diferentes podem andar juntos quando fizer sentido.

---

## Fase 0: entrar no servidor ✅

- [x] RakNet: handshake, confiabilidade, ordenação, fragmentação, ACK/NACK, reenvio
- [x] NetherNet: sinalização HTTP (`/v1/join`), WebRTC (ICE/DTLS/SCTP), identidade ES384 no SDP
- [x] Candidatos ICE: oferecidos pelo cliente + inferidos pelo IP público
- [x] Login até o spawn (NetworkSettings, Login, packs, StartGame, chunks, PlayStatus)
- [x] Leitura de mundos LevelDB (subchunks v1/v8/v9, biomas 3D)
- [x] Multiworld: carregar todas as pastas de `worlds/`
- [x] Cache do batch comprimido de cada chunk, compartilhado entre jogadores
- [x] Workspace organizado em crates, testes com bots em servidor isolado

---

## Fase 1: jogável (em andamento)

### Jogadores

- [x] Lista de jogadores com skin (incluindo persona, capa, animações)
- [x] Ver outros jogadores: AddPlayer, MovePlayer, RemoveActor
- [x] Chat e mensagens de entrar/sair
- [x] Atributos padrão (vida, fome, velocidade...)
- [x] Abilities: voo no criativo
- [x] 🔴 Gamemode por jogador (hoje é um só para o servidor)
- [x] 🟠 Animações: braço balançando (Animate), agachar, correr, nadar (SetActorData por jogador)
- [x] 🟠 Item na mão visível para os outros (MobEquipment)
- [x] 🟠 Armadura visível para os outros (MobArmorEquipment)
- [x] 🟠 Nametag, cor e permissão de operador
- [x] 🟢 Emotes
- [x] 🟢 Trocar skin em jogo (PlayerSkin)

### Blocos

- [x] Quebrar (PlayerAuthInput / criativo), visível para todos
- [x] Colocar (InventoryTransaction), com alcance e espaço livre
- [x] Reenviar o bloco real quando a ação é recusada
- [x] Salvar no LevelDB a cada 5 s (subchunk v8, chunk v42)
- [x] 🔴 Orientação ao colocar: escadas, troncos, baús, fornalhas, portas, lajes
- [x] 🔴 Blocos substituíveis: grama alta, água, lava, neve fina, ar
- [x] 🟠 Colisão: não colocar dentro de jogadores e entidades
- [x] 🟠 Blocos de duas partes: portas (abrem inteiras; de ferro não abrem com a mão), camas (cor no block entity), plantas altas
- [~] 🟠 Lajes duplas ✔, encaixe de cercas, muros e painéis de vidro (estados de conexão)
- [x] 🟠 Tempo de quebra no survival: dureza, ferramenta certa, tier, Eficiência (quebrar rápido demais é recusado; sem drop sem a ferramenta certa) · efeitos (pressa, cansaço) depois
- [x] 🟠 Rachaduras ao quebrar (LevelEvent) e partículas/sons de quebrar e colocar
- [x] 🟢 Pegar bloco com o botão do meio (BlockPickRequest)

### Inventário

- [x] Menu criativo (2221 itens, 128 grupos)
- [x] Abrir e fechar o próprio inventário
- [x] ItemStackRequest: criar, mover, dividir, trocar, descartar, com resposta por slot
- [x] 🔴 Itens reais no servidor: id, meta, NBT, tamanho máximo da pilha
- [x] 🔴 Item na mão (hotbar selecionada, MobEquipment)
- [x] 🟠 Inventário de sobrevivência com regras (sem criar itens do nada)
- [x] 🟠 Armadura e mão secundária
- [x] 🟠 Salvar o inventário do jogador (em NBT por jogador)
- [x] 🟠 Largar itens no chão (entidades de item com física, fusão e coleta)
- [x] 🟢 Craft 2x2 no inventário

### Comandos

- [x] 🔴 Ler CommandRequest e responder (CommandOutput / mensagem)
- [x] 🔴 AvailableCommands: lista e autocompletar ao digitar `/`
- [x] `/gamemode`, `/tp`, `/list`, `/say`, `/help`
- [x] 🔴 `/give` (itens reais no inventário)
- [x] 🔴 `/world` (trocar/criar mundo) e `/worlds` (listar)
- [~] 🟠 `/time` ✔, `/weather` ✔, `/kill` ✔, `/clear` ✔, `/setworldspawn` ✔, `/spawnpoint` ✔, `/difficulty` ✔, `/effect` ✔
- [x] 🟠 `/op`, `/deop`, `/stop`, `/save-all`, `/kick`, `/ban`, `/pardon`, `/whitelist`
- [~] 🟠 Seletores: `@p`, `@a`, `@s`, `@r` ✔ · `@e` com argumentos
- [x] 🟢 Comandos pelo console do servidor (stdin)

### Segurança e conexão

- [x] 🔴 Verificar o login Xbox: token OpenID (RS256, chaves da Mojang), emissor, público, validade e assinatura ES384 dos dados do cliente (`xbox-auth`)
- [x] 🔴 XUID e gamertag verificados (vêm do token assinado pela Mojang)
- [x] 🔴 NetherNet: identidade assinada na oferta (`a=identity`) ligada ao login (mesma `cpk`), impede reusar token de outro (`nethernet-require-identity`)
- [ ] 🟢 Criptografia no RakNet (ECDH P-384 + AES-256-CTR). O Altay já não usa; só vale se o cliente ainda aceitar
- [x] 🟠 Identidade do NetherNet persistente (chave salva em disco)
- [ ] 🟠 TLS na sinalização do NetherNet (o cliente tenta TLS primeiro)
- [x] 🟠 Proteção contra flood: limite de pacotes por segundo, tamanho máximo de batch
- [x] 🟠 Desconectar com motivo traduzido (kick, ban, whitelist, servidor cheio, versão errada)
- [ ] 🟢 Descoberta do NetherNet na LAN (UDP 7551)
- [x] 🟢 Ping RakNet com contagem real e `server-portv6`

### Mundo

- [x] Quebrar/colocar persistentes
- [x] 🔴 Salvar ao desligar (Ctrl+C / `/stop`) sem perder nada
- [x] 🟠 Baús, placas, fornalhas etc. (block entities da chave 0x31) enviados nos chunks + edição de placas
- [x] 🟠 Entidades salvas nos chunks (0x32) preservadas ao salvar
- [x] 🟠 Dia e noite (SetTime, `doDaylightCycle`)
- [x] 🟠 Spawn seguro (achar o chão em vez de y fixo)
- [~] 🟠 Trocar de mundo ✔ e de dimensão (ChangeDimension)
- [x] 🟢 Criar mundo novo vazio/plano pelo servidor (`/world create`)
- [x] 🟢 Ler e escrever `level.dat` (spawn, tempo, gamerules)

---

## Fase 2: survival

### Vida e combate

- [x] Dano: queda ✔, vazio ✔, lava ✔, fogo e queimar ✔, afogamento (bolhas de ar) ✔, sufocar ✔, fome ✔
- [x] Morte e respawn (tela de morte, spawnpoint pessoal, keepInventory)
- [x] Regeneração natural ✔ · fome, saturação, exaustão ✔ (correr, pular, lutar, apanhar, minerar; morrer de fome conforme a dificuldade)
- [x] Combate PvP: dano por item, knockback, crítico, cooldown
- [x] Armadura: proteção ✔ · durabilidade ✔ (gasta ao levar dano, quebra)
- [x] Efeitos (MobEffect): duração, nível, velocidade/lentidão, pressa/cansaço na mineração, força/fraqueza, resistência, absorção, regeneração, veneno, wither, fome, instantâneos; maçã dourada, baiacu, leite
- [~] Encantamentos ✔: Afiada, Repulsão, Proteção, Peso-Pena, Eficiência, Inquebrável, Toque Suave, Fortuna, `/enchant` · Aspecto Flamejante, Pilhagem, Espinhos, mesa de encantamento
- [~] Comida ✔ (tabela vanilla, tempo de comer conferido no servidor, tigela/garrafa de volta) · poções e leite
- [~] Experiência ✔ (níveis vanilla, orbes que voam até o jogador, XP de minérios, perde e solta ao morrer, `/xp`, salvo no jogador) · gasto em encantar/bigorna, XP da fornalha

- [x] Blocos com gravidade: areia, cascalho, pó de concreto, bigorna (entidade caindo, vira bloco ao pousar ou item se o lugar estiver ocupado)

- [x] Água e lava escorrendo: cai primeiro, espalha (água 7, lava 3), seca sem fonte, fonte infinita, obsidiana/pedregulho/pedra quando se encontram

- [x] Drops vanilla dos blocos: minérios (diamante, carvão, ferro bruto...), pedra → pedregulho, grama → terra, vidro/gelo → nada, folhas, cascalho, grama alta; Toque Suave e Fortuna

- [x] Agricultura: sementes, cenoura, batata e beterraba na terra arada, crescimento por random ticks perto dos jogadores, farinha de osso, colheita conforme o estágio; fogo apaga sozinho

### Itens

- [x] Durabilidade de ferramentas e armas (NBT `Damage`, Inquebrável, quebra com som)
- [ ] Arco, besta, tridente, bola de neve, ovo, pérola do fim (projéteis)
- [~] Baldes de água e lava ✔ (encher, esvaziar, pilha de baldes) · leite (vaca), peixes
- [~] Isqueiro ✔, enxada ✔ (terra arada), pá ✔ (caminho), machado ✔ (descascar) · tesoura
- [ ] Livros, mapas, bússola, relógio
- [ ] Fogos de artifício, elytra

### Blocos com interface

- [x] Baú ✔, barril ✔, shulker box ✔ (abrir, mover itens, vários jogadores no mesmo baú, salvar no formato vanilla, derrubar ao quebrar), baú duplo ✔ (pairx/pairz vanilla, 54 slots, cada metade guarda os seus itens)
- [x] Bancada de trabalho e receitas (CraftingData a partir de recipes.json, receita conferida no servidor)
- [~] Fornalha, alto-forno, defumador ✔ (receitas vanilla, combustível, progresso, bloco aceso, formato vanilla) · XP, retomar fornalhas acesas depois de reiniciar
- [ ] Bigorna, mesa de encantamento, rebolo, tear, cortador de pedra
- [ ] Mesa de cartografia, mesa de ferraria (smithing, trims)
- [ ] Funil (hopper), dispensador, ejetor
- [ ] Baú do fim, beacon, suporte de poções

### Física de blocos

- [ ] Areia, cascalho e pó de concreto caindo (entidade de bloco caindo)
- [ ] Água e lava fluindo, fonte infinita, pedra/obsidiana na mistura
- [ ] Fogo espalhando e queimando
- [ ] Plantas: crescimento, colheita, pó de osso, sementes
- [ ] Árvores: mudas crescendo, folhas caindo
- [ ] Gelo derretendo, neve acumulando
- [ ] Grama e micélio espalhando
- [ ] Ticks aleatórios e agendados (salvos em 0x33 / 0x3a)

### Redstone

- [ ] Pó de redstone e propagação de sinal
- [ ] Tocha, repetidor, comparador, alavanca, botões, placas de pressão
- [ ] Pistões e pistões grudentos (movendo blocos e entidades)
- [ ] Portas, alçapões, portões com sinal
- [ ] Lâmpada, observador, sensor de luz, alvo, campainha, trilhos ativadores
- [ ] Bloco de nota, TNT

### Clima e luz

- [ ] Chuva, neve, trovoada (LevelEvent), raios
- [ ] Iluminação (para spawn de mobs e crescimento de plantas)

---

## Fase 3: entidades e mobs

### Base

- [ ] ECS (bevy_ecs ou hecs) para entidades
- [ ] Física: gravidade, colisão com blocos, atrito, água
- [ ] Movimento enviado aos jogadores (MoveActorDelta, SetActorMotion)
- [ ] Itens no chão: jogar, pegar, juntar, sumir após 5 min
- [ ] Orbes de XP
- [ ] Salvar e carregar entidades dos chunks
- [ ] Hitbox e interação (atacar, usar item em entidade)

### Mobs

- [ ] Spawn natural por bioma, luz e limite de mobs
- [ ] Pathfinding (A* em grade) e objetivos de IA (goals)
- [ ] Passivos: vaca, porco, ovelha, galinha (criar, tosquiar, ordenhar)
- [ ] Hostis: zumbi, esqueleto, creeper, aranha, enderman
- [ ] Neutros: lobo, abelha, golem de ferro
- [ ] Aldeões: profissões, trocas, aldeias
- [ ] Mobs do Nether e do End, chefes (Wither, Ender Dragon)
- [ ] Drops e loot tables de mobs
- [ ] Montaria e veículos: cavalo, barco, carrinho de mina

---

## Fase 4: geração de mundo

- [ ] Gerador plano configurável (camadas, bioma)
- [ ] Gerador void (para lobby e minigames)
- [ ] Ruído (Perlin/Simplex) e relevo básico
- [ ] Biomas e distribuição
- [ ] Cavernas, minérios, lagos
- [ ] Árvores e vegetação
- [ ] Estruturas (vilas, templos, fortalezas) a partir de arquivos de estrutura
- [ ] Nether e End
- [ ] Geração em threads separadas
- [ ] Alternativa: pré-gerar no BDS e só carregar (já suportado pelo formato)

---

## Fase 5: plataforma e plugins

### Arquitetura

- [ ] Uma thread de tick por mundo
- [ ] Pool de threads para carregar, comprimir e salvar chunks
- [ ] Barramento de eventos (PlayerJoin, BlockBreak, Chat, Damage...)
- [ ] Eventos canceláveis e prioridades
- [ ] Agendador de tarefas (atraso, repetição, assíncrono)
- [ ] Configuração em YAML/TOML (além do `server.properties`)

### API de plugins

- [ ] Escolher o runtime: WASM (recomendado), Lua ou Rust nativo
- [ ] Carregar, descarregar e recarregar plugins sem reiniciar
- [ ] API: jogadores, mundos, blocos, itens, entidades, comandos, eventos, mensagens
- [ ] Formulários (ModalForm, CustomForm, SimpleForm) e DDUI
- [ ] Scoreboards, bossbar, títulos, actionbar, toasts
- [ ] Armazenamento por plugin (arquivo e banco de dados)
- [ ] Permissões por plugin e por jogador
- [ ] Documentação e plugins de exemplo

### Administração

- [ ] Sistema de permissões (grupos, herança)
- [ ] Whitelist, bans por nome/IP/XUID
- [ ] Console interativo com histórico e autocompletar
- [ ] Logs em arquivo com rotação
- [ ] Métricas: TPS, uso de memória, jogadores, chunks carregados
- [ ] Painel web ou RCON
- [ ] Backup automático de mundos

### Rede e escala

- [ ] Transferir jogador entre servidores (Transfer)
- [ ] Modo proxy / várias instâncias (lobby, rankup, minigames)
- [ ] Resource packs e behavior packs do servidor (enviar e forçar)
- [ ] Itens e blocos customizados (component-based)
- [ ] Entidades customizadas
- [ ] Suporte a várias versões do protocolo ao mesmo tempo (multiversion)
- [ ] Anti-cheat: velocidade, voo, alcance, quebrar rápido, kill aura

---

## Fase 6: qualidade

- [ ] Testes unitários de codec (NBT, varints, paletas, SDP)
- [ ] Testes de integração com bots no CI (GitHub Actions)
- [ ] Builds de release para Windows e Linux no GitHub
- [ ] Imagem Docker
- [ ] Benchmarks: chunks/s, jogadores simultâneos, uso de CPU e memória
- [ ] Fuzzing dos decodificadores de pacote
- [ ] Documentação de uso (instalar, configurar, atualizar)
- [ ] Guia de contribuição

---

## Rankup CeresMC (como plugins, quando a API existir)

- [ ] Economia (moedas, saldo, transferências)
- [ ] Ranks e progressão (rankup)
- [ ] Minas que se regeneram
- [ ] Terrenos (plots) e proteção de área
- [ ] Lojas (NPC, placas, formulários)
- [ ] Kits, homes, warps, tpa
- [ ] Clãs/facções
- [ ] Leilão e mercado
- [ ] Cosméticos (skins, partículas, efeitos de kill)
