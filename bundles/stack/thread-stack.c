#define _GNU_SOURCE
#include <pthread.h>
#include <sys/resource.h>

__attribute__((constructor)) static void thread_stack_default(void) {
  struct rlimit stack;
  pthread_attr_t attr;
  if (getrlimit(RLIMIT_STACK, &stack) || stack.rlim_cur != RLIM_INFINITY || pthread_getattr_default_np(&attr))
    return;
  pthread_attr_setstacksize(&attr, THREAD_STACK_SIZE);
  pthread_setattr_default_np(&attr);
  pthread_attr_destroy(&attr);
}
