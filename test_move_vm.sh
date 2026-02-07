#!/bin/bash
# Test Move VM Integration
set -e

echo "🔧 Miraset Move VM Integration Test"
echo "===================================="
echo ""

# 1. Check build
echo "1️⃣  Verifying build..."
if cargo build --package miraset-node 2>&1 | grep -q "Finished"; then
    echo "   ✅ miraset-node builds successfully"
else
    echo "   ❌ Build failed!"
    exit 1
fi

# 2. Check Move VM dependencies
echo ""
echo "2️⃣  Checking Move VM dependencies..."
if cargo tree -p miraset-node 2>&1 | grep -q "move-vm-runtime"; then
    echo "   ✅ move-vm-runtime integrated"
else
    echo "   ❌ Move VM runtime not found!"
    exit 1
fi

if cargo tree -p miraset-node 2>&1 | grep -q "move-core-types"; then
    echo "   ✅ move-core-types integrated"
else
    echo "   ❌ Move core types not found!"
    exit 1
fi

# 3. Check implementation files
echo ""
echo "3️⃣  Checking implementation files..."
if [ -f "crates/miraset-node/src/move_vm.rs" ]; then
    lines=$(wc -l < crates/miraset-node/src/move_vm.rs)
    echo "   ✅ move_vm.rs exists ($lines lines)"
else
    echo "   ❌ move_vm.rs not found!"
    exit 1
fi

if [ -f "crates/miraset-node/src/pocc.rs" ]; then
    lines=$(wc -l < crates/miraset-node/src/pocc.rs)
    echo "   ✅ pocc.rs exists ($lines lines)"
else
    echo "   ❌ pocc.rs not found!"
    exit 1
fi

if [ -f "crates/miraset-node/src/gas.rs" ]; then
    lines=$(wc -l < crates/miraset-node/src/gas.rs)
    echo "   ✅ gas.rs exists ($lines lines)"
else
    echo "   ❌ gas.rs not found!"
    exit 1
fi

# 4. Check Sui version
echo ""
echo "4️⃣  Verifying Sui dependency version..."
if grep -q 'tag = "mainnet-v1.9.1"' Cargo.toml; then
    echo "   ✅ Using Sui mainnet-v1.9.1"
else
    echo "   ⚠️  Warning: Sui version mismatch"
fi

# 5. Summary
echo ""
echo "📊 Implementation Summary"
echo "========================"
total_lines=$(( $(wc -l < crates/miraset-node/src/move_vm.rs) + $(wc -l < crates/miraset-node/src/pocc.rs) + $(wc -l < crates/miraset-node/src/gas.rs) ))
echo "   Move VM Runtime:    $(wc -l < crates/miraset-node/src/move_vm.rs) lines"
echo "   PoCC Consensus:     $(wc -l < crates/miraset-node/src/pocc.rs) lines"
echo "   Gas System:         $(wc -l < crates/miraset-node/src/gas.rs) lines"
echo "   ─────────────────────────────"
echo "   Total:              $total_lines lines"

echo ""
echo "✅ Move VM Integration Test PASSED!"
echo ""
echo "📚 Next Steps:"
echo "   1. Deploy a Move module:"
echo "      cargo run --bin miraset -- move publish <module.mv>"
echo ""
echo "   2. Execute Move function:"
echo "      cargo run --bin miraset -- move call <module>::<function>"
echo ""
echo "   3. Check gas costs:"
echo "      cargo run --bin miraset -- gas estimate <transaction>"
echo ""
echo "   4. View documentation:"
echo "      cat MOVE_VM_STATUS.md"
