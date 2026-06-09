> Detailed, sequenced proposal for these items lives in [PLAN.md](./PLAN.md).
> Cross-cutting decisions: configurable level sources, a tagged persistence
> layer, and whole-sim determinism with a record/replay fallback.

- Game states + menus
- Loading state while assets stream in?
- debug tooling
- refactor adversary to allow for different kinds (appearance, positioning and behaviors)
  -- static guard, moves their vision cone across in a sweep but doesnt move until their interest threshold is met to cause them to chase
  -- patrolling guard, moves in a set patrol, with similar vision sweeping as they walk and the same interest threshold for chasing
  -- wandering guard, similar the most to the current behavior
  -- any randomness should be calculated using the seed so that the behaviour is identical across loops
- experiments related to time loop events occuring in future loops - do we immediately apply certain things vs have things happen in real time?
- tidy up world generation to be a few rooms of differing types (start, lobby, game tables, vault, security) and have some associated things going on
- after world generation need to look at different mechanics: get code to get into vault from employee who wont leave until certain things happen, etc
- resize game to fit browser window?
- how will the timeloop tick recording handle rigged/animated objects?
