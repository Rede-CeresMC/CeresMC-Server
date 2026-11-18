# Referências

Projetos open source usados como referência. Quando um trecho é portado de um deles, o comentário no código diz de onde veio.

| Projeto | Linguagem | Licença | Para que serve aqui |
|---|---|---|---|
| [Altay](https://github.com/altayofficial/Altay) | PHP | LGPL-3.0 | Layout exato dos pacotes do protocolo atual, fluxo de login, formato LevelDB, NetherNet (rede `altayofficial/Network`). |
| [altayofficial/Network](https://github.com/altayofficial/Network) | PHP | GPL-3.0 | RakNet e NetherNet: sinalização HTTP, identidade SDP, enquadramento de mensagens. |
| [PowerNukkitX](https://github.com/PowerNukkitX/PowerNukkitX) | Java | LGPL-3.0 | Comportamento vanilla já implementado: entidades e IA, redstone, geração de mundo, comandos. |
| [Endstone](https://github.com/EndstoneMC/endstone) | C++ / Python | Apache-2.0 | Desenho da API de plugins (estilo Bukkit/PocketMine), eventos e permissões. |
| [bedrock-data](https://github.com/pmmp/BedrockData) | dados | CC0 | Paletas de blocos e itens, biomas, receitas. Copiados em `data/`. |

## Como usar as referências

Clone ao lado do repositório, fora dele:

```sh
mkdir ../references && cd ../references
git clone --depth 1 https://github.com/altayofficial/Altay.git
git clone --depth 1 https://github.com/altayofficial/Network.git
git clone --depth 1 https://github.com/PowerNukkitX/PowerNukkitX.git
git clone --depth 1 https://github.com/EndstoneMC/endstone.git
```

Licenças: este repositório é GPL-3.0, porque o RakNet e o NetherNet foram portados do `altayofficial/Network` (GPL-3.0). Código do Altay e do PowerNukkitX (LGPL-3.0) e do Endstone (Apache-2.0) é compatível com GPL-3.0 e pode ser portado; mantenha o aviso de copyright original no arquivo portado.
