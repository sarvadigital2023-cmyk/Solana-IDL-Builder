use anchor_lang::prelude::*;

declare_id!("Fg6PaFpoGXkYsidMpWTK6W2BeZ7FEfcYkg476zPFsLnS");

#[program]
pub mod test_program {
    use super::*;
    pub fn initialize(ctx: Context<Initialize>) -> Result<()> {
        msg!("IDL build test OK");
        Ok(())
    }
}

#[derive(Accounts)]
pub struct Initialize {}
