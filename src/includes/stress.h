#ifndef STRESS_H
#define STRESS_H

/* `mon -stress`: start the preemption stress tasks, or check them (all progressed,
 * no register corruption); `mon -stress stop` ends them. See stress.c. */
void stress_command(const char* args);

#endif /* STRESS_H */
