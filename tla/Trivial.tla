---- MODULE Trivial ----
EXTENDS Naturals
VARIABLE x
Init == x = 0
Next == x' = (x + 1) % 3
Spec == Init /\ [][Next]_x
TypeOK == x \in 0..2
====
