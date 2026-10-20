#!/usr/bin/env bash
# 一键发布 ImTipLite 到 GitHub。
#
# 用法:
#   bash tools/publish.sh <owner> [repo]        # 第一次发布(创建仓库并推送)
#   bash tools/publish.sh <owner> [repo] --push # 之后只推送新提交
#
# 例子:
#   bash tools/publish.sh luwenxiang95
#   bash tools/publish.sh luwenxiang95 ImTipLite
#
# 需要先完成 GitHub 认证, 任选其一:
#   - 已安装 gh:  gh auth login
#   - HTTPS + PAT: git config --global credential.helper manager   (弹窗输入 PAT)
#   - SSH:        把 ~/.ssh/id_*.pub 加到 GitHub → Settings → SSH keys
set -euo pipefail

OWNER="${1:-}"
REPO="${2:-ImTipLite}"
PUSH_ONLY=0
[[ "${3:-}" == "--push" ]] && PUSH_ONLY=1

if [[ -z "$OWNER" ]]; then
  echo "用法: bash tools/publish.sh <owner> [repo] [--push]" >&2
  exit 1
fi

cd "$(dirname "$0")/.."

# 1) 修正 README 里的 OWNER 占位符
if grep -q "github.com/OWNER/" README.md 2>/dev/null; then
  sed -i "s|github.com/OWNER/|github.com/$OWNER/|g" README.md
  git add README.md
  git commit -q -m "docs: 指向 $OWNER/$REPO" || true
  echo "[1/4] README 徽章已指向 $OWNER/$REPO"
else
  echo "[1/4] README 徽章已配置, 跳过"
fi

# 2) 配置 remote
URL="https://github.com/$OWNER/$REPO.git"
if git remote get-url origin >/dev/null 2>&1; then
  git remote set-url origin "$URL"
else
  git remote add origin "$URL"
fi
echo "[2/4] remote origin = $URL"

# 3) 首次发布: 用 gh 创建仓库(可选)
if [[ $PUSH_ONLY -eq 0 ]] && command -v gh >/dev/null 2>&1; then
  if ! gh repo view "$OWNER/$REPO" >/dev/null 2>&1; then
    gh repo create "$OWNER/$REPO" --public \
      --description "输入法状态光标跟随提示(中/英 · 全角/半角 · CapsLock)" \
      --source . --remote origin --push
    echo "[3/4] 已创建仓库并推送"
    exit 0
  fi
fi
echo "[3/4] 仓库已存在或未安装 gh, 直接推送"

# 4) 推送
git push -u origin main
echo "[4/4] 完成 → $URL"

echo
echo "下一步(可选): 把绿色版作为 Release 附件上传"
echo "  gh release create v0.1.0 ImTipLite.exe WebView2Loader.dll \\"
echo "     --title 'v0.1.0' --notes '首个版本'"
