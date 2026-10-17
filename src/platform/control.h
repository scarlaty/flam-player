#ifndef CONTROL_H
#define CONTROL_H

/* Pilotage du player par une application hote (option --stdin-control).
 *
 * Un thread lit stdin ligne a ligne : la ligne "quit", ou la fin du flux
 * (stdin ferme, hote termine), demande une sortie volontaire. La boucle
 * principale la traite comme une fermeture de fenetre : etat sauvegarde,
 * code de sortie 0. Sans l'option, stdin n'est jamais lu (un stdin nul ou
 * une console ne doivent pas arreter le player). */

/* Demarre le thread de lecture. 0 si demarre, -1 sinon. */
int control_stdin_start(void);

/* 1 si une sortie a ete demandee sur stdin, 0 sinon. */
int control_quit_requested(void);

#endif /* CONTROL_H */
