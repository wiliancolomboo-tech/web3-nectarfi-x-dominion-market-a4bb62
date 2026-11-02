# Initialize market with 6% Senior APR, 20% FLCR, and 1-day epoch
   anchor run initialize-market -- \
     --market-id 101 \
     --apr-bps 600 \
     --min-flcr 2000 \
     --duration 86400