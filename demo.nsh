# Nimble-Shell Automation Script
echo "=== Welcome to Nimble-Shell Automated Demo ==="
pwd
sysinfo
agents list
echo "Running quick 1-second benchmark with 4 subagents..."
bench --compute --agents 4 --duration 1
echo "=== Demo Complete ==="
