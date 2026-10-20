#!/usr/bin/env zsh
# lackey.zsh OUT.txt CMD...: run CMD under valgrind's lackey, writing each 256-byte line it touched once, as
# "I hexline" (a fetch) or "D hexline" (a load or store). The raw trace goes through a fifo and never reaches the
# disk. stdin and stdout are CMD's, so it can stand in for a viewer in a b2 job.
out=$1; shift
fifo=$out.fifo; rm -f $fifo; mkfifo $fifo
LC_ALL=C awk '/^(I| [LSM]) /{a=substr($2,1,index($2,",")-1); p=substr(a,1,length(a)-2); k=($1=="I"?"I":"D") " " p; if(!(k in s)){s[k]=1; print k; fflush()}}' < $fifo > $out &
valgrind --tool=lackey --trace-mem=yes --log-file=$fifo "$@"
rc=$?; wait; rm -f $fifo; exit $rc
