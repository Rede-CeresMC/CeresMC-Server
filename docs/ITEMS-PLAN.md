# Itens: plano e progresso

Objetivo: cada item do jogo com o comportamento vanilla (usar, consumir, atacar, minerar, construir),
item por item. Referências estudadas: PowerNukkitX (LGPL-3.0, referência principal a partir de agora), df-mc/dragonfly (MIT, `server/item`, `server/block`),
PocketMine-MP (LGPL, só leitura), Mojang bedrock-samples (behavior pack em `data/behavior/`),
bedrock-protocol-docs. Clones locais em `c:\tmp\refs`.

Testes de bot: `tools/weapons_test.py`, `tools/projectiles_test.py`, `tools/mobs_test.py` (rodam em
`tools/run_tests.sh`).

## Lote 1: armas (feito)

- [x] Espadas, machados, picaretas, pás: dano (conferido com o dragonfly: nível + bônus + 1 do punho)
- [x] Enxadas: dano corrigido (madeira/ouro 3, pedra/cobre 4, ferro 5, diamante 6, netherite 7; antes era 1)
- [x] Lanças: dano conferido no `item_components.nbt` (`minecraft:damage` 1/2/2/3/1/4/5 para madeira/pedra/cobre/ferro/ouro/diamante/netherite, mais 1 do punho); a estimativa anterior estava 1 acima. Falta o ataque de investida
- [x] Maça: ataque esmagador (4/bloco nos 3 primeiros, 2/bloco nos 5 seguintes, 1/bloco depois; Densidade +0,5 por nível e bloco), sem dano de queda, empurra quem está perto, Rajada de Vento joga o atacante para cima, som leve/pesado
- [x] Besta: segurar 1,25 s carrega (Carga Rápida reduz 0,25 s por nível), a flecha fica no NBT `chargedItem` (todos veem armada), usar de novo dispara; Multitiro = 3 flechas; gasta durabilidade
- [x] Tridente: segurar e soltar arremessa (8 de dano, +2,5 por nível de Empalamento em criaturas da água), crava no bloco, pode ser recolhido com encantamentos e durabilidade, Lealdade faz voltar; Correnteza lança o jogador na água ou na chuva
- [x] Flechas (jogador e mobs): apontam na direção do voo e cravam no bloco por 60 s; as do jogador se recolhem andando por cima
- [x] Besta com foguete: um foguete na mão secundária é carregado antes das flechas (vai para o `chargedItem` com o seu `Fireworks`), sai em linha reta e explode no que acertar ou no fim do voo; Multitiro solta três; gasta 3 de durabilidade. Sem teste de bot (o bot não põe item na mão secundária)
- [x] Perfuração: a flecha da besta atravessa até N entidades (nível N) ferindo cada uma uma vez e segue voando (teste: atravessa um porco e fere o jogador atrás)
- [x] Raios (`lightning_bolt`): 5 de dano e fogo a até 3 blocos, fogo no chão (com `mob-griefing`); nos mobs a causa `lightning` aciona os sensores do pack (porco vira piglin zumbi, creeper carregado, aldeão vira bruxa...). Raios naturais nas tempestades perto dos jogadores; `/summon lightning_bolt`
- [x] Canalização: tridente que acerta algo a céu aberto numa tempestade chama um raio
- [x] Apelidos de identificador que os arquivos do pack ainda usam (`pig_zombie`, `evoker`...)
- [x] Carga da lança (lógica do PowerNukkitX `ItemSpear.whileUsing`, LGPL-3.0): segurando o uso e movendo-se a 0,13 bloco/tick ou mais, a cada 5 ticks acerta o mais próximo à frente com 1,5x o dano + 3 por bloco de velocidade (sem teste de bot)
- [ ] Golpe com Estocada (Lunge, encantamento 41: impulso para a frente gastando fome)

## Lote 2: ferramentas (em andamento)

IDs conferidos: sons em `bedrock-protocol-docs/json/LevelSoundEvent.json`, ActorEvent em `ActorEvent.json`, LevelEvent no gophertunnel (`level_event.go`; a fumaça de explosão estava com o ID 2052, inexistente, agora 2025). Sons conferidos contra `bedrock-protocol-docs/json/LevelSoundEvent.json` (corrigidos `random.pop` e `random.enchant`, que não existem).

Já existiam: enxada (terra arada), pá (caminho), machado (descascar tronco), isqueiro (fogo), tesoura (folhas, ovelha), baldes de água/lava/leite, farinha de osso.

- [x] Machado: raspar oxidação e cera do cobre (todos os blocos de cobre: blocos, cortados, escadas, lajes, portas, alçapões, lâmpadas, grades, correntes, baús, estátuas)
- [x] Favo de mel: encerar cobre
- [x] Favo de mel em placas (encera: ninguém edita mais)
- [x] Oxidação do cobre com o tempo (random tick, cerca de uma hora por estágio)
- [x] Corantes: tingir ovelha (troca o grupo de cor do behavior pack)
- [x] Corantes, tinta brilhante e tinta em placas (no lado voltado para o jogador)
- [x] Ovos de spawn (91): criam o mob do behavior pack no chão (nome do ovo vira nome do mob); ovo usado no mesmo mob cria um filhote
- [x] Ovo de spawn no spawner; spawners funcionam (até 4 mobs a cada 10-40 s com jogador a 16 blocos, máximo 6 por perto)
- [x] Baldes de peixe/axolote/girino: despejam a água e soltam o mob; pegar com balde de água vem dos arquivos do pack (`minecraft:interact`)
- [x] Carga de fogo (acende fogo), chifre de cabra (som pela variante, recarga de 7 s), carga de vento (projétil do pack com rajada; serve para impulso)
- [x] Vara de pesca: boia com linha, espera de 5-30 s (Isca reduz), fisgada com bolhas, loot de pesca do pack com Sorte do Mar (qualidade das entradas), XP, puxar mob/jogador fisgado, durabilidade
- [x] Pesca na selva: com a boia num bioma com a tag `jungle` usa `jungle_fishing.json`
- [ ] Peixe nadando até a boia (efeito visual)
- [ ] Pincel: areia/cascalho suspeitos (o behavior pack de exemplo não traz as loot tables de arqueologia; falta fonte para o loot)
- [x] Olho do fim na moldura do portal do End: encaixa o olho; com os 12 olhos no anel em volta de um buraco 3x3 o portal abre (som `block.end_portal.spawn`)
- [ ] Olho do fim lançado no ar (sem fortalezas ainda); viajar pelo portal (sem o End); luneta, bússola e relógio são do cliente
- [x] Montaria: montar mobs cujo `minecraft:rideable` aceita jogadores (porco selado, cavalo, lavagante selado...), vínculo SetActorLink (41), o mob segue o assento do jogador (o cliente dirige: movimento autoritativo do servidor com previsão do veículo), sinal WASD_CONTROLLED/CAN_POWER_JUMP; descer com Interact StopRiding (3), ao morrer, sair ou teleportar
- [x] Sela e armadura de cavalo (`minecraft:equippable`); sela do porco pelo `minecraft:interact` do pack; domar cavalo montando (`minecraft:tamemount`: aceita ou derruba)
- [ ] Barcos e carrinhos de mina (não são mobs com vida), passageiros extras, pulo do cavalo
- [ ] Laço (guia)
- [ ] Mapas (vazio, preenchido)

## Lote 3: construção e uso de blocos

- [ ] Revisar colocação de todos os blocos com estados (orientação, meia-laje, escadas, portas, camas já feitos)
- [x] Plantas crescendo sozinhas (random tick): cana e cacto (até 3 blocos), bambu (12-16), alga, caules de melancia/abóbora (dão o fruto ao lado e se viram para ele), arbusto de frutas silvestres, cacau, fungo do Nether, broto de flor-tocha e de jarro
- [x] Farinha de osso: caules, frutas silvestres, cacau, flor-tocha/jarro, cana e bambu (crescem a coluna toda, como no Bedrock), alga, grama baixa e samambaia viram duplas, flores altas soltam uma cópia, bloco de grama faz brotar grama e flores em volta
- [x] Mudas viram árvores (random tick: 1 em 7, o primeiro só marca a idade; farinha de osso: 45 %): carvalho, bétula, selva, abeto, acácia inclinada e cerejeira (copa simplificada). Formas portadas do dragonfly (`sapling_tree.go`, MIT), que segue as features de árvore do Bedrock
- [x] Carvalho escuro e carvalho pálido: 4 mudas em quadrado crescem um tronco 2x2 com a copa achatada (`roofedCanopy` do dragonfly, sem os galhos)
- [x] Abeto e selva gigantes: 4 mudas em quadrado (`megaTrunk` do dragonfly): abeto com copa de pinheiro e podzol em volta, selva com copa em camadas e galhos
- [x] Cipós na árvore da selva (`treeVines` do dragonfly: tronco 33 % com 1 bloco, copa 25 % com até 4 blocos pendurados)
- [x] Azaleia (e florida) com farinha de osso vira árvore de azaleia: tronco de carvalho de 4-5 que entorta no alto, terra com raízes embaixo, folhas de azaleia (1 em 4 floridas)
- [x] Mangue: o propágulo plantado passa pelos estágios 0-4 (random tick ou farinha de osso) e vira um mangue simplificado: raízes em arco, tronco elevado, copa redonda e propágulos pendurados
- [x] Farinha de osso em cogumelos (40 %): cogumelo gigante marrom (chapéu 7x7) ou vermelho (domo 5x5), estipe e chapéu com `huge_mushroom_bits`
- [x] Farinha de osso no bloco de musgo: o musgo cobre pedra e terra em volta e brota tapete de musgo, grama e azaleias
- [x] Farinha de osso debaixo d'água: grama marinha vira dupla; no fundo (areia, cascalho, terra, argila, lama) brota grama marinha em volta
- [x] Cipós de caverna: farinha de osso faz nascer bagas brilhantes; usar o cipó colhe as bagas
- [x] Arbusto de frutas silvestres: usar colhe (1-2 no estágio 2, 2-3 no 3) e volta ao estágio 1; com farinha de osso na mão ele cresce antes
- [x] Farinha de osso: folha pingente pequena vira grande (caule + folha), a grande cresce um bloco; picles do mar sobre coral na água enche até 4 (sem teste de bot)
- [x] Corais (blocos, plantas, leques) morrem sem água em volta ou na camada de água do próprio bloco (nova `World::get_block_layer`)
- [x] Blocos alagados: um bloco que guarda água (lajes, escadas, cercas, muros, vidraças, baús, placas, lanternas, alçapões, folhas, corais, velas...) colocado em água parada fica com a água na segunda camada (UpdateBlock camada 1); quebrado, deixa a água
- [x] Balde de água num bloco que guarda água o alaga; balde vazio num bloco alagado tira a água
- [x] Água de blocos alagados escorre para os lados e para baixo (o bloco alagado conta como fonte que nunca é substituída); balde vazio tira a água sem quebrar o bloco
- [x] Líquen brilhante: farinha de osso cria um novo líquen num bloco de ar vizinho, preso a uma face sólida (bits do `multi_face_direction_bits` conferidos no PocketMine; sem teste de bot)
- [x] Balde de neve em pó (coloca e recolhe neve em pó)
- [x] Sementes restantes: melancia, abóbora, flor-tocha e vagem de jarro na terra arada; fungo do Nether na areia das almas; frutas silvestres na terra/grama; cacau na lateral do tronco da selva (direção como no dragonfly `cocoa_bean.go`)
- [x] Velas e picles do mar empilham até 4 no mesmo bloco; isqueiro/carga de fogo acende velas, bolo com vela e fogueiras; pá apaga fogueira; mão vazia apaga velas; vela em bolo inteiro vira bolo com vela da mesma cor
- [x] Bolo: comer fatia a fatia (2 de fome, 0,4 de saturação, 7 fatias)
- [ ] Alga e picles do mar na água, bagas brilhantes (cipó de caverna), cogumelos com regra de luz
- [x] Livro e pena: páginas guardadas no NBT (pacote BookEdit 97: trocar, adicionar, apagar, trocar de lugar; até 50 páginas de 256 caracteres) e assinar vira livro escrito com título, autor e geração 0
- [x] Molduras (e brilhantes): usar coloca o item da mão, usar de novo gira (8 posições), bater tira o item, quebrar solta o item (lógica do dragonfly `item_frame.go`, sons por LevelEvent 1040/1043/1044)
- [x] Púlpito: recebe livro e pena ou livro escrito, virar páginas (LecternUpdate 125) é guardado para todos, bater devolve o livro, quebrar também
- [x] Suporte de armadura: o item coloca a entidade do pack virada para o jogador (passos de 45°); usar com item veste (o que estava volta), mão vazia tira o item da mão ou a peça mais alta; quebrar solta o suporte (loot table do pack) e tudo o que vestia
- [x] Poses do suporte de armadura: agachar e usar passa para a próxima das 13 poses (metadata PoseIndex 79), salva com o mob
- [x] Metadata conferida no gophertunnel (`entity_metadata.go`): todas as chaves usadas batem; agora também SkinID (104, aparência dos aldeões) e ColorTwoIndex (82, segunda cor do peixe tropical)
- [ ] Copiar livro escrito (receita), mapas em molduras

## Lote 4: consumíveis

Comida, poções, totem e garrafas já existiam.

- [x] Fruta do coro: teleporta para um lugar livre a até 8 blocos (até 16 tentativas), com som
- [x] Garrafa de mel: cura o veneno
- [x] Ensopado suspeito: efeito da flor pelo valor auxiliar (visão noturna, salto, fraqueza, cegueira, veneno, saturação, resistência ao fogo, regeneração, wither, náusea)
- [x] Recargas de item (pacote PlayerStartItemCooldown, 176; categorias tiradas de `item_components.nbt` e do PocketMine): pérola do fim 1 s, fruta do coro 1 s (`chorusfruit`), chifre de cabra 7 s, carga de vento 0,5 s, escudo desativado por machado 5 s. O servidor também impede o uso durante a recarga
- [x] Recarga de ataque das lanças (componente `minecraft:cooldown`, categoria `spear`: madeira 0,65 s, pedra 0,75, cobre 0,85, ferro/ouro 0,95, diamante 1,05, netherite 1,15); golpes durante a recarga são ignorados
