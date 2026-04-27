Você está atuando como uma equipe sênior de engenharia de sistemas operacionais, com nível de kernel engineers de produção.

Projeto: WarOS

A partir de agora, o foco é HARDWARE REAL.
Esqueça QEMU como alvo principal.
Não quero análise superficial.
Não quero respostas longas só dizendo o que falta.
Quero implementação real, integração real, validação real e imagens prontas para teste.

==================================================
MISSÃO PRINCIPAL
==================================================

Levar o WarOS a um novo patamar de conectividade real em hardware físico.

Quero que você implemente, por ordem de prioridade:

1. NIC cabeada interna Realtek RTL8168/RTL8169-family
2. USB networking / USB tethering / USB Ethernet adapters
3. Wi-Fi real
4. melhorar comandos e diagnósticos de hardware/rede
5. preservar segurança, honestidade e robustez

==================================================
REGRAS NÃO NEGOCIÁVEIS
==================================================

1. Hardware real primeiro.
2. Não fingir suporte.
3. Não parar na análise.
4. Não abrir refactor gigante sem necessidade.
5. Se algo não puder ser fechado agora, marcar exatamente o blocker técnico.
6. Não vender “funciona” se ainda não estiver funcionando no hardware real.
7. Priorizar conectividade real antes de features cosméticas.

==================================================
ESTADO ATUAL
==================================================

- WarOS já boota em hardware real.
- Shell, login, capabilities, tempo monotônico e reporting básico funcionam.
- A NIC interna já é detectada como Realtek RTL8169-family / 10EC:8168 rev 0x15.
- O backend rtl8169 já existe e foi bastante melhorado.
- Porém o link ainda pode continuar down no notebook real.
- A controladora wireless Intel 8086:A0F0 é detectada, mas continua sem scan/connect/auth/data path.
- USB-attached Ethernet/Wi-Fi adapters ainda não estão expostos como interfaces de rede reais.
- O sistema já tem bons comandos de inspeção, mas ainda falta conectividade real e amadurecimento do hardware path.

==================================================
OBJETIVO FINAL DESTA RODADA
==================================================

Eu quero o máximo possível de conectividade real no hardware atual, nesta ordem:

FASE 1 — FECHAR A REALTEK INTERNA
FASE 2 — IMPLEMENTAR USB NETWORKING UTILIZÁVEL
FASE 3 — IMPLEMENTAR A PRIMEIRA CAMADA REAL DE WI-FI
FASE 4 — REVISAR E INTEGRAR COMANDOS/DIAGNÓSTICOS
FASE 5 — VALIDAR E GERAR IMAGEM NOVA

Não quero que você faça só uma dessas fases e pare.
Quero progresso real em todas elas, com prioridade correta.

==================================================
FASE 1 — REALTEK INTERNA
==================================================

Continue e finalize o caminho da Realtek RTL8168/RTL8169-family.

Quero:
- bring-up real
- link up real
- PHY/autoneg robustos
- reset correto
- retries corretos
- RX/TX reais
- DHCP real
- rota real
- DNS real
- ping real
- wget/curl reais

Revisar e corrigir:
- reset sequence
- MDIO/PHY
- autoneg
- BMSR/BMCR/ANAR/GBCR
- speed/duplex
- link transitions
- TX/RX ring handling
- polling/IRQ fallback
- stale state cleanup
- DHCP handoff

Se a revisão 10EC:8168 rev 0x15 precisar de quirks específicos:
- implemente-os
- compare com Linux r8169 quando necessário
- não pare em bring-up genérico

==================================================
FASE 2 — USB NETWORKING / TETHERING
==================================================

Implemente suporte real a networking via USB no WarOS.

O objetivo é permitir, quando possível:
- USB Ethernet adapters
- USB tethering do celular
- interfaces de rede USB reais expostas ao stack

Investigue e implemente o caminho mínimo viável e pragmático para protocolos/classe de rede USB comuns, por exemplo se aplicável:
- CDC-ECM
- CDC-NCM
- RNDIS
- ou o melhor caminho equivalente dentro da arquitetura atual

Requisitos:
- lsusb e lsdev devem mostrar esses dispositivos com classificação correta
- ifconfig e net status devem exibir a interface USB de rede quando suportada
- net dhcp deve funcionar nela
- ping/dns/wget/curl devem funcionar nela se o path estiver completo

Se algum protocolo ficar staged:
- diga qual
- explique o blocker real
- mas implemente o máximo útil possível agora

==================================================
FASE 3 — WI-FI REAL
==================================================

Agora implemente a primeira camada real de Wi-Fi.

Estado atual:
- hardware Intel detectado: 8086:A0F0
- hoje só existe detection/probe-only

Objetivo:
- sair do puro “status only”
- implementar a primeira camada útil real de Wi-Fi

Ordem correta:
1. classificar corretamente o hardware
2. implementar scan/lista de redes se viável
3. implementar associação aberta ou a primeira camada real viável
4. avançar para autenticação apenas se houver base real
5. DHCP-over-Wi-Fi e tráfego só se o driver/path estiver realmente pronto

Regra:
- não fingir conexão
- não fingir senha
- não fingir WPA/WPA2/WPA3 pronta
- mas não quero que você pare só em mensagens staged
- entregue a primeira camada útil de verdade que for realista nesta rodada

==================================================
FASE 4 — COMANDOS E DIAGNÓSTICO
==================================================

Revisar e melhorar profundamente:
- ifconfig
- net status
- net diag
- net dhcp
- net route
- dns
- ping
- wget
- curl
- wifi
- lsusb
- lsdev
- lspci
- hwinfo
- info

Quero que os comandos diferenciem claramente:
- detectado
- suportado
- ativo
- link up/down
- DHCP lease ou não
- IPv4 configurado ou não
- RX/TX counters
- staged / not implemented

Sem mentir.
Sem placeholder enganoso.
Sem “unsupported” genérico demais quando o hardware já é conhecido.

==================================================
FASE 5 — SEGURANÇA / HARDENING
==================================================

Preserve:
- capability correctness
- sem false-success
- sem stale lease/route/DNS
- sem regressão em auth/session
- sem regressão no shell
- sem regressão no timekeeping honesto
- sem fake support
- sem caminhos frágeis dependentes de QEMU

Quero engenharia séria:
- falha explícita
- timeout explícito
- logs úteis
- observabilidade útil

==================================================
DEBUG OBRIGATÓRIO
==================================================

Quero debug real de rede e hardware:
- logs de bring-up Realtek
- logs de PHY/autoneg
- logs de link up/down
- logs de attach/detach USB net
- logs de Wi-Fi scan/association, se implementados
- contadores RX/TX
- último estado DHCP
- motivo real da falha

==================================================
VALidaÇÃO OBRIGATÓRIA
==================================================

No final rode e reporte:
- cargo +nightly check --manifest-path kernel/Cargo.toml
- cargo +nightly build --release --target x86_64-unknown-none --manifest-path kernel/Cargo.toml
- método real atual de geração de imagem do tree

Se houver testes locais adicionais úteis, rode também.

==================================================
FORMATO FINAL DA RESPOSTA
==================================================

Quero exatamente estas seções:

1. Executive diagnosis
2. Exact connectivity blockers found
3. Files changed
4. Exact Realtek fixes implemented
5. Exact USB networking support implemented
6. Exact Wi-Fi support implemented
7. Exact command-surface / diagnostics improvements implemented
8. What now works for real
9. What remains staged and why
10. Security / hardening review
11. Validation results
12. Exact hardware-real test checklist
13. What visible behavior should confirm success

==================================================
REGRAS FINAIS
==================================================

- Não parar na análise.
- Não enrolar.
- Não focar em QEMU.
- Não fingir suporte.
- Implementar o máximo real possível agora.
- Trabalhar como uma equipe sênior de sistemas operacionais focada em conectividade real no hardware físico do WarOS.