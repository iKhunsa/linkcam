# Builds the release executable and assembles dist\LinkCam (exe + gesture models).
$ErrorActionPreference = 'Stop'
Set-Location $PSScriptRoot
cargo build --release -p linkcam-app -p linkcam-vcam
$dist = Join-Path $PSScriptRoot 'dist\LinkCam'
New-Item -ItemType Directory -Force $dist, "$dist\models" | Out-Null
Copy-Item target\release\linkcam.exe $dist -Force
Copy-Item target\release\linkcam_vcam.dll $dist -Force
Copy-Item models\*.onnx "$dist\models" -Force
New-Item -ItemType Directory -Force "$dist\samples" | Out-Null
Copy-Item crates\linkcam-vision\tests\data\open_palm.jpg "$dist\samples" -Force
Copy-Item ..\LICENSE "$dist\LICENSE" -Force
Copy-Item models\MODELS-LICENSE.txt "$dist\models" -Force -ErrorAction SilentlyContinue
Write-Host "Packaged to $dist"
Get-ChildItem $dist -Recurse -File | Select-Object FullName, @{n='MB';e={[math]::Round($_.Length/1MB,2)}}
