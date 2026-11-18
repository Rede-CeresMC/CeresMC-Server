# Roadmap

## Fase 0: entrar no servidor ✅
- [x] RakNet
- [x] NetherNet (sinalização HTTP, WebRTC, identidade ES384)
- [x] Login até o spawn
- [x] Leitura de mundos LevelDB e envio de chunks
- [x] Multiworld (carregamento)

## Fase 1: jogável (em andamento)
- [ ] Lista de jogadores com skin, ver outros jogadores se movendo, chat
- [ ] Voo no criativo (UpdateAbilities)
- [ ] Quebrar e colocar blocos, salvando no LevelDB
- [ ] Inventário, menu criativo, ItemStackRequest
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
