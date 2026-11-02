# Calculate future maturity timestamp: current + 7776000 seconds (90 days)
MATURITY=$(($(date +%s) + 7776000))
SCALAR=10000

# Invoke initialize_market
anchor run init-market -- --maturity $MATURITY --scalar $SCALAR