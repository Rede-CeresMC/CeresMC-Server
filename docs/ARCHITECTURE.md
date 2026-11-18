# Arquitetura

## Crates

```
ceres-server ──► ceres-net ──► ceres-protocol ──► ceres-codec
     │                                ▲
     └──────► ceres-world ────────────┘
```

| Crate | Responsabilidade |
|---|---|
| `ceres-codec` | `W`/`R` (buffers com varints, LE/BE) e NBT nos três formatos: big-endian (arquivos de dados), little-endian (mundo) e rede (varints). |
| `ceres-protocol` | Constantes do protocolo, codec de batch (deflate + prefixo de algoritmo) e os construtores de pacotes, separados por fase em `packets/`. A ordem dos campos segue o `bedrock-protocol` do Altay. |
| `ceres-net` | A trait `Transport` e suas implementações: `raknet/` (UDP) e `nethernet/` (WebRTC, com sinalização HTTP em TCP na mesma porta). |
| `ceres-world` | `data/`: registros vanilla carregados uma vez (paleta de blocos, estados/orientação, itens, pilha máxima, biomas, voxel shapes); pacotes estáticos já pré-codificados. `world/`: mundos LevelDB, block entities (`0x31`), `level.dat`, criação de mundos planos/void e multiworld. |
| `ceres-server` | Binário `ceres`: configuração, loop principal, sessões, inventário real + equipamentos, entidades de item, combate/vida/respawn, blocos/placas, comandos e moderação. |

## Rede

Os dois transportes entregam eventos iguais para o jogo (`NetEvent::Connected / Packet / Disconnected`). A camada de jogo nunca sabe por onde o jogador entrou.

- **RakNet** (`ceres-net/src/raknet`): handshake offline, frames confiáveis/ordenados, fragmentação, ACK/NACK e reenvio. Pacotes do jogo vão atrás do byte `0xFE`.
- **NetherNet** (`ceres-net/src/nethernet`):
  1. `GET /v1/join` em TCP devolve o status JSON (lista de servidores).
  2. `POST /v1/join/{networkId}` recebe a oferta SDP e devolve a resposta. A resposta leva `a=identity`, um JWS ES384 sobre os fingerprints DTLS; sem ele o cliente recusa ("você não foi convidado").
  3. ICE + DTLS + SCTP via crate `webrtc`. O jogo usa o canal `ReliableDataChannel`. Cada mensagem tem 1 byte com o número de segmentos que ainda faltam; o payload remontado é o mesmo batch do RakNet, sem o `0xFE`.

## Mundos

- Formato LevelDB do Bedrock (compressão zlib id 2 / raw deflate id 4), lido com `rusty-leveldb`.
- Os subchunks são convertidos para o formato de rede sem decodificar os blocos: as palavras compactadas são copiadas e só a paleta é traduzida (estado NBT → id de rede com hash, vindo do `block_palette.nbt`).
- O batch comprimido de cada `LevelChunk` fica em cache por mundo e é compartilhado entre jogadores; o cache é limpo quando ninguém está perto.
- Mundos planos: chunks que não existem no disco usam o chunk do spawn como molde.

## Loop principal

Uma thread de jogo: lê eventos dos transportes, processa pacotes, roda o tick a cada 50 ms (envio de chunks) e despacha os batches de saída. O NetherNet roda num runtime tokio próprio e conversa com o loop por canais.

## Planejado

- Uma thread de tick por mundo e um pool para gerar, comprimir e salvar chunks.
- Entidades em ECS.
- Barramento de eventos para a API de plugins.
