# Roadmap

## Fase 0: entrar no servidor ✅
- [x] RakNet
- [x] NetherNet (sinalização HTTP, WebRTC, identidade ES384)
- [x] Login até o spawn
- [x] Leitura de mundos LevelDB e envio de chunks
- [x] Multiworld (carregamento)

## Fase 1: jogável (em andamento)
- [x] Lista de jogadores com skin, ver outros jogadores se movendo, chat
- [x] Voo no criativo (UpdateAbilities)
- [x] Quebrar e colocar blocos, salvando no LevelDB (orientação de blocos, colisão e blocos substituíveis pendentes)
- [x] Menu criativo, abrir o inventário, pegar/mover/dividir/trocar/descartar itens (ItemStackRequest com resposta por slot)
- [ ] Itens reais no inventário do servidor (ids, NBT, tamanho máximo de pilha), sobrevivência, baús, salvar inventário
- [ ] Comandos: `/tp`, `/gamemode`, `/give`, `/world`
- [ ] Verificação Xbox / token de identidade
- [ ] Criptografia no RakNet

## Fase 2: survival
- [ ] Drops e loot tables
- [ ] Crafting e fornalha
- [ ] Vida, fome, dano, queda
- [ ] Dia/noite e clima
- [ ] Física de blocos (areia, água, lava)
- [ ] Redstone básica

## Fase 3: entidades
- [ ] ECS
- [ ] Itens no chão, XP
- [ ] Spawn de mobs, IA e pathfinding

## Fase 4: plataforma
- [ ] Uma thread por mundo, pool de chunks
- [ ] API de plugins
- [ ] Permissões
- [ ] Várias instâncias (lobby / modos de jogo)
- [ ] Anti-cheat
