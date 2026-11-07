#!/bin/sh
# 更新翻译：从源码重新提取 msgid，合并进 en.po，编译 .mo。
# 新增/修改界面文案后跑一次；en.po 里留空的条目会回落到中文。
set -e
cd "$(dirname "$0")/.."

xgettext --from-code=UTF-8 -ktr -o po/cyanrip-frontend.pot crates/cyanrip-gui/src/*.rs 2>/dev/null
msgmerge --quiet --update po/en.po po/cyanrip-frontend.pot
msgfmt -o po/en/LC_MESSAGES/cyanrip-frontend.mo po/en.po
mkdir -p "$HOME/.local/share/locale/en/LC_MESSAGES"
cp po/en/LC_MESSAGES/cyanrip-frontend.mo "$HOME/.local/share/locale/en/LC_MESSAGES/"

missing=$(msgattrib --untranslated po/en.po | grep -c '^msgid ' || true)
echo "已编译 po/en/LC_MESSAGES/cyanrip-frontend.mo（未翻译条目：$missing）"
