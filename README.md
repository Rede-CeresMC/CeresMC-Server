# CeresMC Server

Servidor de **Minecraft: Bedrock Edition** escrito em **Rust**, da Rede CeresMC.

> A Minecraft: Bedrock Edition server written in Rust, aiming for vanilla behaviour with a plugin API.

**Estado:** em desenvolvimento inicial. Já dá para entrar no servidor e andar pelo mundo; o gameplay ainda está sendo implementado (veja o [roadmap](docs/ROADMAP.md)).

| | |
|---|---|
| Versão do Minecraft | Bedrock 1.26.60 (protocolo 2225) |
| Transportes | RakNet (UDP) e NetherNet (WebRTC), na mesma porta |
| Mundos | LevelDB vanilla, vários mundos carregados ao mesmo tempo |
| Licença | [GPL-3.0](LICENSE) |

## O que já funciona

- Aparece na lista de servidores (RakNet e NetherNet).
- Login completo até o spawn, pelos dois transportes.
- Lê mundos vanilla (LevelDB) e envia os chunks; o chunk já comprimido fica em cache e é compartilhado entre jogadores.
- Vários mundos carregados a partir de `worlds/`.

## Compilar e rodar

Requer Rust 1.85+ (edition 2024).

```sh
cargo build --release
cp server.properties.example server.properties
mkdir worlds            # coloque aqui pastas de mundo com level.dat + db/
./target/release/ceres
```

O servidor procura `data/`, `worlds/` e `server.properties` na pasta onde é executado.
`CERES_DEBUG=1` liga logs detalhados de rede.

Teste rápido sem abrir o jogo (precisa do servidor rodando):

```sh
python tools/smoke_client.py
```

## Estrutura

```
crates/
  ceres-codec/      buffers binários, varints e NBT (BE / LE / rede)
  ceres-protocol/   pacotes do Bedrock e codec de batch
  ceres-net/        transportes: RakNet e NetherNet (WebRTC + sinalização HTTP)
  ceres-world/      dados vanilla (blocos, itens, biomas) e mundos LevelDB
  ceres-server/     binário `ceres`: sessões, login, envio de chunks
data/               dados vanilla do bedrock-data (CC0)
docs/               arquitetura, roadmap e referências
tools/              ferramentas de teste
```

Detalhes em [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md).

## Referências

O formato dos pacotes e várias decisões vêm de outros servidores open source. Veja [docs/REFERENCES.md](docs/REFERENCES.md).

## Licença

[GNU GPL v3.0](LICENSE). Os arquivos em `data/` vêm do [bedrock-data](https://github.com/pmmp/BedrockData) e são CC0.
