# Arborescence d'une installation : une ligne `<chemin relatif> <SHA-256>` par fichier, triée,
# sans uninstall.exe (régénéré par l'installateur). Chargée par points (`. ./scripts/Get-InstallTree.ps1`)
# dans les étapes de la CI qui comparent une installation propre et une mise à jour.
function Get-InstallTree([string]$Dir) {
  Get-ChildItem -LiteralPath $Dir -Recurse -File | Where-Object { $_.Name -ne 'uninstall.exe' } |
    ForEach-Object { '{0} {1}' -f $_.FullName.Substring($Dir.Length), (Get-FileHash -LiteralPath $_.FullName -Algorithm SHA256).Hash } |
    Sort-Object
}
