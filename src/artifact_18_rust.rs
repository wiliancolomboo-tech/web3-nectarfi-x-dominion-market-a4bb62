require!(
       updated_nectar_index >= market.last_yield_index, 
       DominionError::YieldIndexRegressed
   );